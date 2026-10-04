# Chat product and UX gap analysis

Area key `chat-ux`. 15 verified findings. Part of the [foks-rs review](../README.md).

## Current state

The desktop chat is careful about integrity and weak on everyday use. On the integrity side it keeps a durable send journal (intent, prepare, attempt, reconcile), verifies history pages before showing them, quarantines channels that fail verification, and keeps the read pointer monotonic and gated on focus. The UX around that is still MVP-level. Rendering covers a small, bounded markdown subset (message-text.tsx), groups messages by author within 5 minutes, adds day separators and a single "NEW" divider, and offers one action: Copy message, reachable only by right-click. When a channel opens, the view jumps to the bottom and the whole channel is marked read 300 ms later (use-chat-read-intent.ts). A busy channel left open eventually hits the 1,000-row view cap and throws an error (conversation-model.ts). The channel list is ordered by the server inbox_version, which also moves whenever you read a channel. List rows show no preview or time, even though the README and an unused helper (previewLine) describe them. The "Muted" and "Hidden" captions can never appear against this server, because it always sends false for both. Sender names come only from the current roster, so a departed member's messages show as a shortened hex ID. Unsent and failed sends appear in three different layouts (OutgoingRow, the in-thread PendingRow, and the team-level "Needs attention" section). Rejections show raw numeric codes such as "(12006)", and send errors pass through the agent's developer-facing strings. There is no message search, no channel-switching shortcut, no role="log" live region, and no focusable message actions. What the IPC already provides: history paging with before/after cursors, per-message verification status, read_through, unread counts, previews, and read/write roles. Edits, reactions, threads, mentions, channel rename/delete and server-side mute have no protocol method, or are rejected for non-Basic message types (server-db realtime/messages.rs:16). ISSUES.md defers these as extended chat. Most of the refinements below therefore fit in desktop-ui on today's data, with a few small agent/IPC additions. The mock build (chat-mock.ts) seeds one channel and one message with a null sender, so screenshots show a "Team member" label that production never produces.

## Index

| Finding | Type | Priority | Effort |
|---|---|---|---|
| [Channel list: stable ordering, previews and times, draft and unread emphasis, persisted folds, honest context menu](#chat-ux-channel-list) | feature-refinement | high | M |
| [Open channels at the first unread message, mark read by what was actually seen, and count arrivals while scrolled up](#chat-ux-open-at-first-unread) | feature-refinement | high | M |
| [Admins-only channels overstate their audience; show the real readers and the roles in channel info](#chat-ux-channel-audience) | correctness-risk | medium | S |
| [Composer refinements: formatting affordances, mention autocomplete, draft indicators, Up-to-edit-unsent, character feedback](#chat-ux-composer) | feature-refinement | medium | M |
| [One legible delivery and integrity vocabulary: unify unsent-message rows, map rejection codes, and mark verification per message](#chat-ux-delivery-states) | feature-refinement | medium | M |
| [Message search: start with find-in-conversation over decrypted history, then an agent-side encrypted index](#chat-ux-history-search) | missing-feature | medium | L |
| [Replace the hard 1,000-row view cap with a sliding window, and auto-load older pages](#chat-ux-history-window-sliding) | correctness-risk | medium | M |
| [Keyboard navigation and screen-reader support for the conversation](#chat-ux-keyboard-a11y) | feature-refinement | medium | M |
| [Markdown link labels can disguise the destination; show the host and confirm mismatches](#chat-ux-link-label-spoofing) | security | medium | S |
| [Make mute real: a device-local per-channel and per-team mute that drives badges, plus a 'Mentions only' alert mode](#chat-ux-local-mute) | missing-feature | medium | M |
| [Add a hover/focus action bar per message: copy, quote-reply, copy link, details (no protocol change)](#chat-ux-message-actions) | missing-feature | medium | M |
| [Message rendering: autolink bare URLs, keep paragraph breaks, style code blocks, highlight @mentions](#chat-ux-message-rendering) | feature-refinement | medium | S |
| [Resolve every sender to a verified username, including departed members, and mark former members](#chat-ux-sender-identity) | feature-refinement | medium | M |
| [Split ChatThread into header, list, row and composer, and derive display rows as a pure model](#chat-ux-thread-decomposition) | maintainability | medium | M |
| [Give the chat mock realistic scenarios so design review and screenshots reflect production](#chat-ux-mock-fidelity) | tooling | low | S |

### chat-ux-channel-list

**Channel list: stable ordering, previews and times, draft and unread emphasis, persisted folds, honest context menu**

- Type: feature-refinement
- Priority: high
- Effort: M
- Layers: desktop-ui, docs
- Verification: adjusted

Column order is whatever the agent returns. That is conversations by inbox_version DESC, and the server assigns a new inbox_version when the user's own read pointer advances (and on their own sends). So reading a channel moves it above channels that still have unread messages after the next sync, and the list reorders under the cursor. Rows show only '#name' and a count. They have no last-message preview or time, although the inbox carries previews, presentation.previewLine exists but is unused, and the desktop README still documents previews. Team collapse state resets on remount. The channel context menu permanently shows disabled 'Edit channel' and 'Delete channel', and has no 'Mark as read', 'Mute' or 'Copy link'. The header's 'New chat' button opens 'Create channel' with 'Choose a team' even when a team is open, and only one team has chat.

**Evidence**

- [`crates/foks-client-db/src/soft.rs:432`](../../../crates/foks-client-db/src/soft.rs#L432): SELECT … FROM chat_inbox_channels … ORDER BY inbox_version DESC
- [`crates/foks-server-db/src/realtime/inbox.rs:376`](../../../crates/foks-server-db/src/realtime/inbox.rs#L376): rt_read_through assigns next_inbox_version to the channel row when read_through rises, and wakes the actor
- [`apps/desktop/src/chat/presentation.ts:143`](../../../apps/desktop/src/chat/presentation.ts#L143): listChannels keeps agent order: conversations, then never-messaged channels, then hidden
- [`apps/desktop/src/chat/presentation.ts:221`](../../../apps/desktop/src/chat/presentation.ts#L221): previewLine is exported but has no caller in src or tests
- [`apps/desktop/src/screens/chat-teams.tsx:334`](../../../apps/desktop/src/screens/chat-teams.tsx#L334): collapsedTeams is component state; comment: 'a team reopens expanded the next time the column mounts'
- [`apps/desktop/src/screens/chat-teams.tsx:589`](../../../apps/desktop/src/screens/chat-teams.tsx#L589): 'Edit channel' and 'Delete channel' MenuItems are always disabled with 'not implemented' reasons
- [`apps/desktop/README.md:697`](../../../apps/desktop/README.md#L697): documents the row's last message ('You: …', 'sam.ortiz: …'), its time, a 'Conversations' label and 'nothing folds away', none of which match chat-teams.tsx
- [`apps/desktop/src/screens/group-tabs.tsx:202`](../../../apps/desktop/src/screens/group-tabs.tsx#L202): comment claims the Chat column draws the same preview stamp; it does not

**Recommendation**

Desktop-UI and docs. (1) Sort in listChannels instead of inheriting agent order. By default, put general first, then sort alphabetically by channelLabel. Add a column option 'Sort: Alphabetical / Recent activity', where recent activity uses preview.insert_time and never inbox_version. Keep hidden channels last. (2) Add a second row line with previewLine(conversation, actor, partyNames) and previewTime, behind a 'Show previews' density toggle stored with the sidebar prefs. (3) Persist collapsed team ids with the sidebar-prefs.ts guarded localStorage pattern. (4) In the context menu, remove the permanently disabled Edit and Delete items until channel management ships (ISSUES.md:187). Add 'Mark as read', which calls mark-read with lastPosition(conversation), and 'Mute channel' (see chat-ux-local-mute). Do not add 'Copy link to channel' until the app registers an external URL scheme whose links carry no local profile or alias identifiers. (5) Make the header button preselect the open team, or the only chat-capable team, and name all entry points 'New channel'. (6) Update apps/desktop/README.md (≈690-760: the 'Conversations' label, 'nothing folds away', 'No chat', and the preview wording) and the group-tabs.tsx:201-202 comment to match the shipped column.

**Already tracked:** Channel rename/delete is deferred (ISSUES.md 'Extended channel management … remain disabled'); the ordering, preview, fold and menu changes here are not tracked

**Mockup:** [Chat channel column](../mockups/chat-channel-column.html)

<details><summary>Verifier note</summary>

Most claims hold. soft.rs:432-435 orders by inbox_version DESC. server inbox.rs:376-386 assigns next_inbox_version when read_through rises. soft.rs:341-350 upserts the higher version on the next sync, so a channel just read jumps to the top. listChannels (presentation.ts:143-161) keeps agent order, and nothing in the desktop or agent re-sorts. previewLine (221) has no caller anywhere in apps/desktop. Channel rows render only the name, caption, lock and count (chat-teams.tsx:910-981). collapsedTeams is component state (334-346). Edit and Delete are permanently disabled (589-601). The header 'New chat' calls openNewChat without a team (chat-tab.tsx:129-130), and chat-new.tsx does not auto-select a single team. The README (690-705) documents previews, a 'Conversations' label and 'nothing folds away'. The code shows 'Teams' (chat-teams.tsx:513) and 'Chat unavailable' rather than 'No chat', and folding exists. The group-tabs.tsx:201-202 comment is stale. The column '+' says 'New channel' while the topbar says 'New chat'. ISSUES.md:187-188 tracks only rename/delete. One part of the recommendation is not feasible as written: 'Copy link to channel'. No OS URL scheme or deep-link plugin is registered in the Tauri config. In-app locations encode local StoreRefs (profile, alias, team id), which mean nothing outside this installation and expose local identifiers. That item should be dropped or made conditional on a future app URL scheme.

</details>

### chat-ux-open-at-first-unread

**Open channels at the first unread message, mark read by what was actually seen, and count arrivals while scrolled up**

- Type: feature-refinement
- Priority: high
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

Opening a channel always scrolls to the bottom, and 300 ms later the newest message is marked read if the window has focus. A user with 40 unread messages sees only the last screen, and the rest are marked read without having been viewed. A history page is 50 rows, so with more than 50 unread the first unread message is not even loaded. The NEW divider is computed once per mount and never cleared (it was still showing above a 2023 message after sending in the mock). The jump-to-latest control is a bare chevron that does not say how many messages arrived while the user was scrolled up.

**Evidence**

- [`apps/desktop/src/chat/use-chat-viewport.ts:11`](../../../apps/desktop/src/chat/use-chat-viewport.ts#L11): followBottom starts true; the layout effect at 39-41 sets scrollTop to scrollHeight on each messages/pendingKey change while followBottom is true
- [`apps/desktop/src/chat/use-chat-read-intent.ts:40`](../../../apps/desktop/src/chat/use-chat-read-intent.ts#L40): schedule() takes messages.at(-1) (line 42) and calls markRead 300 ms later (52-62) whenever enabled && atBottom && focusedWindow(), whatever rows were on screen
- [`apps/desktop/src/chat/use-chat-read-intent.ts:31`](../../../apps/desktop/src/chat/use-chat-read-intent.ts#L31): setNewFrom((old) => old ?? readThrough): set once and never cleared for the life of the mount
- [`apps/desktop/src/chat/chat-thread.tsx:406`](../../../apps/desktop/src/chat/chat-thread.tsx#L406): Jump to latest is an icon-only chevron Button shown whenever !atBottom, with no count of arrivals
- [`crates/foks-agent-proto/chat-limits.json:4`](../../../crates/foks-agent-proto/chat-limits.json#L4): CHAT_PAGE_ROWS is 50
- [`crates/foks-agent/src/chat.rs:644`](../../../crates/foks-agent/src/chat.rs#L644): end = before-1; read_chat_thread(end-width+1 .. end), so an anchored window needs no IPC change
- [`crates/foks-client/src/realtime/history.rs:89`](../../../crates/foks-client/src/realtime/history.rs#L89): read_after gap covers truncation, head < after (rollback) and short ranges alike, and a truncated reply carries the tail rows, not the rows after the cursor
- [`crates/foks-server-db/src/realtime/inbox.rs:376`](../../../crates/foks-server-db/src/realtime/inbox.rs#L376): read_through is updated only when the new sequence is greater, so marking read in partial steps is safe
- [`apps/desktop/src/chat/visibility.ts:5`](../../../apps/desktop/src/chat/visibility.ts#L5): messageVisible() already tests whether a [data-message] row intersects the viewport

**Recommendation**

Desktop-UI only. (1) Anchored open: when the conversation has unread > 0, make the first request `history {before: String(read_through + 41)}`, through a distinct 'anchor' load mode in useChatHistory. Do not reuse load(older): that path calls viewport.capture(older) and conversationResult's prepend logic. Scroll the NEW divider to about 30% from the top. (2) Give HistoryWindow a `detached` flag, set when the held tail is below the published head. While detached, make no incremental `after` reads. Page forward with explicit range reads (`before = held_tail + 51`) behind a 'Load newer messages' sentinel, and clear `detached` when a page reaches the published head. Keep treating gap=true from an `after` read as a reset, because it also signals rollback and omitted rows. (3) Mark read by visibility: an IntersectionObserver on [data-message] rows tracks the highest sequence that has been at least 50% visible for 500 ms while focusedWindow(), and that sequence goes to markRead in place of messages.at(-1). The server pointer is monotonic. (4) When the divider is above the viewport, pin a banner reading 'N new messages since <time> · Jump · Mark as read (Esc)'. Clear newFrom on Esc, on mark-all, or when leaving the channel. (5) Replace the chevron with a 'N new messages ↓' pill that counts rows newer than the highest sequence held when atBottom became false. (6) Add tests in chat.render.test.tsx for anchored open with unread=120, visibility-based marking, the pill count, and a rollback gap still resetting while detached.

**Mockup:** [Open at first unread](../mockups/chat-unread-anchored-open.html)

<details><summary>Verifier note</summary>

The core claims hold. useChatViewport starts with followBottom=true and the layout effect (32-43) pins the scroller to the bottom. useChatReadIntent marks messages.at(-1) about 300 ms after mount whenever atBottom && focusedWindow(). newFrom is set once with `old ?? readThrough` and never cleared. Jump to latest is an icon-only chevron (chat-thread.tsx:406-416). CHAT_PAGE_ROWS is 50. The agent accepts an arbitrary `before` (chat.rs:644, end = before-1, start = end-49), and the server GetThread does not reject an end past the head. The server read pointer is monotonic: rt_read_through updates only when the new sequence is greater. Neither ISSUES.md nor the book tracks any of this. Two cited line numbers are wrong: use-chat-read-intent.ts:85 is cleanup code, because schedule() runs from line 40 and marks `latest` at 42/52-62, and newFrom is set at line 31, not 76. Recommendation step (2) is unsafe as written. read_after (foks-client history.rs:71-98) sets gap=true for truncation, for a sequence rollback (head < after) and for omitted rows. Treating every gap=true as 'more newer rows exist' would hide a reset. Also, a gapped reply carries the tail page, not the rows after the cursor. Separately, a detached window would set off an incremental tail read on every revision, because publishedHead is always past the held tail. load(older) in useChatHistory and viewport.capture also treat any `before` request as an older-page prepend, which sets atBottom=false and anchors the scroll offset. The anchored open therefore needs its own load mode.

</details>

### chat-ux-channel-audience

**Admins-only channels overstate their audience; show the real readers and the roles in channel info**

- Type: correctness-risk
- Priority: medium
- Effort: S
- Layers: desktop-ui
- Verification: adjusted

For admins-only channels, the header member count (shown only when the channel has no description), the empty-state sentence 'Only the N members of <team> can read it', and the info panel's Members row all count the whole team roster, although the read role is Admin. A member-tier channel whose read role is a band above some members' bands is overcounted the same way. The info panel shows only 'Visibility', although read_role and write_role are in the contract, and it links to Teams instead of listing who can read the channel.

**Evidence**

- [`apps/desktop/src/screens/chat-screen.tsx:159`](../../../apps/desktop/src/screens/chat-screen.tsx#L159): memberCount = partiesOf(snapshot, storeId).length || undefined, with no filter by channel roles
- [`apps/desktop/src/chat/chat-thread.tsx:283`](../../../apps/desktop/src/chat/chat-thread.tsx#L283): empty state: `Only the ${plural(memberCount,'member')} of ${teamName} can read it.`; header count shown only without a description (171-175)
- [`apps/desktop/src/screens/chat-info.tsx:65`](../../../apps/desktop/src/screens/chat-info.tsx#L65): parties = whole team roster sorted by roleRank; Members row shows parties.length (163)
- [`apps/desktop/src/model/roles.ts:60`](../../../apps/desktop/src/model/roles.ts#L60): admits(held, need) already compares rank and Member visibility band
- [`apps/desktop/src/model/readers.ts:97`](../../../apps/desktop/src/model/readers.ts#L97): readersOf already filters the roster by an item's read role via admits; the chat audience can follow it
- [`crates/foks-agent/src/chat.rs:22`](../../../crates/foks-agent/src/chat.rs#L22): role_label emits 'Owner' / 'Admin' / 'Member (n)'
- [`crates/foks-server-db/src/realtime/policy.rs:73`](../../../crates/foks-server-db/src/realtime/policy.rs#L73): server read policy: role >= tier floor && role >= read role
- [`apps/desktop/src/chat/presentation.ts:208`](../../../apps/desktop/src/chat/presentation.ts#L208): roleTextWithoutBand has no callers

**Recommendation**

Desktop-UI only. Add channelAudience(snapshot, storeId, channel) next to readersOf in model/readers.ts (or in chat/presentation.ts). It normalizes 'Member (n)' to {role:'Member', visibility:n}, keeps the user parties whose destination_role satisfies admits(destination_role, read_role), and marks the subset that also satisfies admits(destination_role, write_role). Use the reader count in the header sub-line when there is no description, in the empty-state sentence ('Only the 3 admins and owners of Engineering can read it'), and in the info panel. In the panel, add the rows 'Who can read' and 'Who can post', and a reader list with role and a 'can post'/'read only' tag. Keep 'Manage in Teams' for membership changes. Either use roleTextWithoutBand for the role labels or delete it. Add render tests for an admin-tier channel in a 6-member team with 2 admins, and for a member-tier channel with a banded read role.

**Mockup:** [Channel readers and alerts](../mockups/chat-channel-info-and-alerts.html)

<details><summary>Verifier note</summary>

Core claim holds. chat-screen.tsx:159-161 counts every party on the store. chat-thread.tsx:283-287 renders 'Only the N members of <team> can read it' whatever the tier. chat-info.tsx:65-68 and 163 show the whole roster under 'Members'. ChatChannel carries read_role/write_role (chat-contract.ts:112-113), which the agent formats as 'Owner'/'Admin'/'Member (n)' (foks-agent/src/chat.rs:22-28). roleTextWithoutBand has no callers. The server admits a reader only when role >= tier floor and role >= read (foks-server-db/src/realtime/policy.rs:73-100). Not tracked in ISSUES.md or the book. Four corrections: (1) The header shows the count only when the channel has no description (chat-thread.tsx:171-175; the render test at chat.render.test.tsx:257 relies on this). (2) The recommended rank-only comparison is wrong for Member bands. model/roles.ts:60 already has `admits(held, need)`, which compares bands, and model/readers.ts:97 `readersOf` already filters a roster by read role for KV items. The helper should reuse both, after normalizing 'Member (n)' to {role:'Member', visibility:n}. (3) The overcount also affects member-tier channels, for example a Go-created channel with read Member(n>0), or members whose visibility band is negative. (4) Limiting the list to user parties matches ISSUES.md:172, which supports only direct same-host user membership.

</details>

### chat-ux-composer

**Composer refinements: formatting affordances, mention autocomplete, draft indicators, Up-to-edit-unsent, character feedback**

- Type: feature-refinement
- Priority: medium
- Effort: M
- Layers: desktop-ui, agent
- Verification: adjusted

The composer is a bare auto-growing textarea. Enter sends and Shift/Alt+Enter insert a newline, but nothing tells the user so. There is no formatting help for the markdown subset the renderer supports, no Ctrl+B/I/E, and no @-completion of roster names. The size meter appears only above 75% of 64 KiB and reads in KiB. Per-channel drafts survive channel switches in the send service's in-memory map, but the channel list does not show which channels have a draft, and drafts are lost on app lock or quit because only submitted intents are persisted. An unsent or queued message can be restored for editing only through a small 'Edit' link.

**Evidence**

- [`apps/desktop/src/chat/chat-thread.tsx:433`](../../../apps/desktop/src/chat/chat-thread.tsx#L433): onKeyDown: Enter sends, Alt+Enter inserts a newline, Shift+Enter falls through to the default; no other shortcuts, no hint text
- [`apps/desktop/src/chat/chat-thread.tsx:465`](../../../apps/desktop/src/chat/chat-thread.tsx#L465): meter rendered only when nearLimit (>75%) as '<n> KiB of 64 KiB'
- [`apps/desktop/src/chat/send-service.ts:425`](../../../apps/desktop/src/chat/send-service.ts#L425): drafts(storeId) is an in-memory Map per team; setDraft at 439 has no persistence
- [`apps/desktop/src/chat/outgoing-row.tsx:92`](../../../apps/desktop/src/chat/outgoing-row.tsx#L92): restoring an unsent message is an 'Edit' text button; service.restoreDraft exists (send-service.ts:1114)
- [`apps/desktop/src/chat/message-text.tsx:88`](../../../apps/desktop/src/chat/message-text.tsx#L88): supported inline subset: `code`, **bold**, *italic*, `[label](url)`; block: ``` fences, - / 1. lists, > quotes

**Recommendation**

Desktop-UI first. (1) Add a hint row under the composer: 'Enter to send · Shift+Enter for new line · **bold** *italic* `code`'. Show it on focus, with a dismissible '?' popover listing exactly the subset message-text.tsx renders. (2) Ctrl/Cmd+B, I and E wrap the selection in **, * and `; a further binding wraps it in a ``` fence (check that it does not collide with webview devtools shortcuts). (3) @-autocomplete from partyNames(snapshot, storeId), inserting '@username'. For admin-tier channels, rank or filter by channel readability. This is plain Basic text. (4) Up arrow in an empty composer calls restoreDraft on the newest not-sent or queued OutgoingMessage in this channel. (5) Show the remaining budget from 90% in the unit the limit uses ('3.3 KB left'), or as an approximate 'about n characters left' computed from the bytes. Keep the byte check authoritative for disabling Send. (6) Draw a pencil glyph on channel rows in chat-teams.tsx when sends.draft(storeId, channel.id) is non-empty. (7) Optional agent step: separate save-draft and load-draft chat actions stored in the agent's protected store, alongside SaveIntent text, so drafts survive lock and restart. Keep them apart from SaveIntent, which means 'submit this', and keep clearing them on account removal as stop() and team replacement do now.

**Already tracked:** Edit of sent messages and mentions as protocol features are deferred in ISSUES.md 'Existing disclosed limitations'; the plain-text @-completion and Up-to-restore-unsent here need no protocol change

**Mockup:** [Chat channel column](../mockups/chat-channel-column.html), [Chat composer refinements](../mockups/chat-composer.html)

<details><summary>Verifier note</summary>

All cited behaviour holds. onKeyDown (chat-thread.tsx:433-454): Enter sends, Alt+Enter inserts a newline through setRangeText, and Shift+Enter falls through. No hint text exists anywhere in the app. The meter renders only when draftBytes > 0.75 × 65,536 (use-chat-composer.ts:75) and reads 'n KiB of 64 KiB'. Drafts live in an in-memory Map per team (send-service.ts:425-439) and are cleared in stop() (line 253) and when a team is removed or replaced (line 357). restoreDraft exists at 1114 and requires an empty draft. The outgoing row's 'Edit' button is at outgoing-row.tsx:91-99. partyNames exists in presentation.ts:115. chat-teams.tsx renders no draft indicator. SaveIntent text is a SecretString held in the agent. Mentions as a protocol feature are tracked as deferred in ISSUES.md; plain-text @-completion is not. One correction to step (5): the limit is 65,536 UTF-8 bytes, so 'n characters left' cannot be derived exactly from the byte budget and would be wrong for non-ASCII text. Step (3) should also note that partyNames lists every team user, including users who cannot read an admin-tier channel.

</details>

### chat-ux-delivery-states

**One legible delivery and integrity vocabulary: unify unsent-message rows, map rejection codes, and mark verification per message**

- Type: feature-refinement
- Priority: medium
- Effort: M
- Layers: desktop-ui, agent
- Verification: adjusted

Unsent work appears in three layouts. OutgoingRow shows avatar, 'You', status and Retry/Edit. In-thread PendingRow ops render without avatar or author, with 'Message · Not sent' and a 'Details' disclosure showing a truncated operation ID and the raw state. The team-level 'Needs attention' section sits above the pane. A rejected operation reads 'The server rejected this operation (12006)', and other send errors show the agent's developer strings verbatim (for example 'unsupported chat operation: latest message has unsupported content'). Verification gaps get one channel-wide warning band, although HistoryWindow tracks verification per message. 'Channel stopped' and 'Chat stopped' tell users to lock and unlock FOKS. These states are correct but read as alarming and opaque.

**Evidence**

- [`apps/desktop/src/chat/chat-thread.tsx:385`](../../../apps/desktop/src/chat/chat-thread.tsx#L385): pending ops render as a bare article with MessageText + PendingRow, no avatar/header, unlike OutgoingRow at 368-383
- [`apps/desktop/src/chat/pending-row.tsx:27`](../../../apps/desktop/src/chat/pending-row.tsx#L27): rejection copy interpolates the raw numeric rejection_code
- [`apps/desktop/src/chat/pending-row.tsx:88`](../../../apps/desktop/src/chat/pending-row.tsx#L88): Details disclosure shows shortId(op.id) · op.state
- [`crates/foks-client/src/realtime/operations/attempt.rs:130`](../../../crates/foks-client/src/realtime/operations/attempt.rs#L130): definite rejections: 1013, 1030, 12002, 12003, 12005, 12006
- [`crates/foks-agent/src/main.rs:2046`](../../../crates/foks-agent/src/main.rs#L2046): chat errors returned with chat.to_string() as the user-facing message; transport.rs from_agent passes it through
- [`apps/desktop/src/chat/conversation-events.ts:59`](../../../apps/desktop/src/chat/conversation-events.ts#L59): missing = missing_predecessors.length > 0, a page-level boolean
- [`apps/desktop/src/chat/conversation-model.ts:100`](../../../apps/desktop/src/chat/conversation-model.ts#L100): the page-level flag is copied onto every message id of the page; not per-message evidence
- [`crates/foks-agent-proto/src/chat.rs:320`](../../../crates/foks-agent-proto/src/chat.rs#L320): ChatMessage has no previous_sequence, so the desktop cannot map missing predecessors to rows
- [`apps/desktop/src/chat/chat-thread.tsx:202`](../../../apps/desktop/src/chat/chat-thread.tsx#L202): single channel-level band 'Some earlier messages could not be checked…'
- [`apps/desktop/src/screens/chat-screen.tsx:324`](../../../apps/desktop/src/screens/chat-screen.tsx#L324): Channel stopped pane: 'lock and unlock FOKS to revalidate this channel'
- [`apps/desktop/src/screens/chat-screen.tsx:436`](../../../apps/desktop/src/screens/chat-screen.tsx#L436): 'Needs attention' section rendered above the pane for other channels' unsent work

**Recommendation**

(1) Desktop-UI: use a single MessageRow with a delivery slot for incoming rows, OutgoingRow and tracked operations, with the fixed state vocabulary proposed: Sending, Queued, Not sent, Checking delivery, Couldn't post, Cancelled, Waiting for access. Move the operation ID into a Details popover. (2) Either add `rejection_reason` beside rejection_code in ChatOperation (agent-proto chat.rs:359, chat-contract.ts:276), mapped from the attempt.rs closed set, or map that closed numeric set in the desktop. Show the code only in Details. (3) Desktop: keep a copy table keyed on CommandError.code for chat-* codes. Do not show agent `message` text for these codes; keep it in diagnostics. (4) Per-message integrity needs an agent/IPC addition first. Add a per-ChatMessage `predecessor_unchecked: bool`, computed in the agent from each message's metadata.previous_sequence against missing_predecessors, and validate it in chat-contract.ts. Only then mark those rows with a glyph and replace the band with a 'History partly checked' header chip that scrolls to the first marked row. Until then, keep a page-level indicator. (5) Turn 'Needs attention' into a header chip with a count, opening a popover with jump links. (6) For a stopped channel, add 'Recheck channel'. It removes the id from the desktop's team.blocked set, which is sent with sync-inbox, and reloads. The agent re-reports any channel it still detects as conflicting via blocked_channels, and hard-state anchors still catch equivocation. Add 'Copy diagnostic details'.

**Already tracked:** ISSUES.md 'Existing disclosed limitations' covers degraded inbox and bounded anchors (the behaviour, not its presentation); book/16-chat.qmd 'Sending from the client' describes the states

**Mockup:** [Delivery and integrity](../mockups/chat-delivery-and-integrity.html)

<details><summary>Verifier note</summary>

Most claims hold. The three layouts exist. OutgoingRow (outgoing-row.tsx) has an avatar, 'You', status and Retry/Check again/Edit. Tracked ops at chat-thread.tsx:385-403 render a bare article plus PendingRow showing 'Message · Not sent' and a Details disclosure with shortId(op.id) · op.state (pending-row.tsx:82-89). 'Needs attention' is rendered above the pane for other channels' and create ops (chat-screen.tsx:207-213, 436-451). The rejection copy interpolates the numeric code (pending-row.tsx:25-28). definite_rejection is the closed set 1013/1030/12002/12003/12005/12006 (attempt.rs:130-139, names verified in foks-rpc status_codes.rs). Agent chat errors pass chat.to_string() (main.rs:~2046) through transport.rs from_agent unchanged, and the desktop renders it via failure(). The 'latest message has unsupported content' string exists (prepare.rs:227). The 'lock and unlock FOKS' copy is at chat-screen.tsx:305-309 and 330-333. The claim that verification is tracked per message is wrong. conversation-model.ts:100 sets the same page-level value on every message, because `missing` is `result.missing_predecessors.length > 0` (conversation-events.ts:59). ChatMessage in IPC has no previous_sequence (agent-proto chat.rs:320-327). So the desktop cannot tell which row followed an unfetched predecessor. As written, recommendation (4) would mark up to a whole page of rows, and its popover text would be false for most of them. A per-row marker needs an agent/IPC field. Priority is lowered to medium: the states are correct, and the problems are presentation and copy, not correctness.

</details>

### chat-ux-history-search

**Message search: start with find-in-conversation over decrypted history, then an agent-side encrypted index**

- Type: missing-feature
- Priority: medium
- Effort: L
- Layers: desktop-ui, agent, docs
- Verification: adjusted

Message content cannot be searched. The Chat column, and the topbar field that shows a '#<channel>' scope badge, filter only team and channel names, because the agent has no message index and the server holds only ciphertext. A first step needs no new IPC: the open conversation already holds verified, decrypted rows in its HistoryWindow (at most 1,000 rows / 8 MiB, past which further loading fails with a chat-limit error rather than trimming). Search across channels needs an index the agent owns, and that changes the chat design's current rule of keeping no durable plaintext.

**Evidence**

- [`apps/desktop/src/screens/chat-teams.tsx:347`](../../../apps/desktop/src/screens/chat-teams.tsx#L347): 'The column searches names, not messages: the agent has no message index'
- [`apps/desktop/src/chat/conversation-model.ts:89`](../../../apps/desktop/src/chat/conversation-model.ts#L89): past CHAT_HISTORY_ROWS (1000) or CHAT_HISTORY_BYTES (8 MiB) the window throws 'chat-limit' ('Reopen it to load the newest messages'); no sliding trim
- [`apps/desktop/src/shell/topbar.tsx:158`](../../../apps/desktop/src/shell/topbar.tsx#L158): in Chat the search field's scope badge reads '#<channel>', yet the query filters only names in the column
- [`apps/desktop/README.md:743`](../../../apps/desktop/README.md#L743): documents no message-content search, and refers to a 'conversation header's search button' that chat-thread.tsx does not render
- [`crates/foks-agent/src/chat.rs:250`](../../../crates/foks-agent/src/chat.rs#L250): the agent's decrypted preview cache is in-memory only (4096 entries); nothing persists message plaintext
- [`book/16-chat.qmd:301`](../../../book/16-chat.qmd#L301): durable chat metadata uses a keyed MAC so it does not reveal guessable text; an on-disk index departs from this
- [`apps/desktop/src/shell/search-palette.tsx:55`](../../../apps/desktop/src/shell/search-palette.tsx#L55): palette scopes are all/items/stores/people/channels; no messages scope

**Recommendation**

Step 1 (desktop UI, S-M): a find-in-conversation bar (Ctrl/⌘+F, or the topbar field while its scope badge names the channel) that matches case-insensitively over the loaded HistoryWindow, with highlights, 'n of m', Enter/Shift+Enter to step, and scroll-and-flash on the current row. 'Search older messages' must not add pages to the HistoryWindow beyond the CHAT_HISTORY_ROWS/BYTES ceiling. Either scan older pages in a transient pass that keeps only hit references, or first add a trimming policy to conversationResult. Cap one click at about 5 pages (250 rows). Also fix README.md:743-745, which describes a header search button that does not exist. Step 2 (agent, L): an index owned by the agent, filled only from verified pages and scoped by (host, uid, team, channel), and dropped when access to the channel is lost. Choose between an in-memory index discarded on lock and per-channel blobs sealed under the vault key; avoid plain FTS5, whose shadow tables persist plaintext tokens. Record the change to durable-plaintext handling in book/16-chat.qmd before shipping it. Expose a bounded `search` chat action and a 'Messages' palette scope, and state that it covers only messages this device has read.

**Already tracked:** apps/desktop/README.md:743-746 and chat-teams.tsx:347-348 document the absence; this adds a no-IPC first step and an agent index design

**Mockup:** [Chat find and search](../mockups/chat-find-and-search.html)

<details><summary>Verifier note</summary>

Core claim holds. Message content cannot be searched (chat-teams.tsx:347-348, README.md:743-745), the palette has no messages scope (search-palette.tsx:55), and HistoryWindow holds decrypted rows. Two parts of the recommendation are wrong or understated. (1) There is no sliding-window trim. conversationResult throws a 'chat-limit' error once the window passes CHAT_HISTORY_ROWS=1000 or 8 MiB (conversation-model.ts:89-98), and pages are CHAT_PAGE_ROWS=50, so '10 pages per click' fails on the second click. (2) The agent keeps decrypted previews only in memory (foks-agent/src/chat.rs:250-306). The chat design deliberately keeps durable state free of plaintext: the ledger works 'without storing any plaintext' (book/16-chat.qmd:139), and submissions use a MAC rather than a hash so that durable metadata does not reveal guessable text (book/16-chat.qmd:301-306). FTS5 shadow tables store plaintext tokens, and the bundled rusqlite has no SQLCipher, so 'FTS5 encrypted at rest under the vault key' is not feasible as written. New evidence strengthens step 1: in Chat, the topbar search field carries a '#<channel>' scope badge (topbar.tsx:158, 570, 615-624) but filters only team and channel names, so a field labeled with the channel does not search it. README.md:743-745 also mentions a 'conversation header's search button', which the thread header (chat-thread.tsx:169-192, info button only) does not have.

</details>

### chat-ux-history-window-sliding

**Replace the hard 1,000-row view cap with a sliding window, and auto-load older pages**

- Type: correctness-risk
- Priority: medium
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

The held history window throws once it exceeds 1,000 rows or 8 MiB. Paging back past the cap shows 'This conversation view reached its history limit. Reopen it to load the newest messages.' On the tail path, the cache catches that error and replaces the window with the incoming page alone, which can be one row after an incremental read, and a unit test asserts this behaviour. The cache's row and byte limits are also shared across all 16 cached channels, so one channel paged back to the cap evicts every other channel's window. Older history loads only through an explicit button, and scroll position is not restored when the user switches back to a channel.

**Evidence**

- [`apps/desktop/src/chat/conversation-model.ts:90`](../../../apps/desktop/src/chat/conversation-model.ts#L90): messages.length > CHAT_HISTORY_ROWS or bytes > CHAT_HISTORY_BYTES throws code 'chat-limit' instead of trimming
- [`apps/desktop/src/chat/history-cache.ts:156`](../../../apps/desktop/src/chat/history-cache.ts#L156): for before === null, a chat-limit error rebuilds the window from the incoming page alone; for older pages the error propagates to the thread
- [`apps/desktop/src/chat/history-cache.ts:187`](../../../apps/desktop/src/chat/history-cache.ts#L187): rows and bytes are summed across all cached channels and the LRU entries are evicted until the total is at most 1,000 rows / 8 MiB, so one full channel evicts the rest
- [`apps/desktop/tests/chat-history-cache.test.ts:179`](../../../apps/desktop/tests/chat-history-cache.test.ts#L179): this test asserts that a 1,000-row window plus one new row leaves only ['1001']
- [`apps/desktop/src/chat/use-chat-history.ts:156`](../../../apps/desktop/src/chat/use-chat-history.ts#L156): latest.current keeps a separate, untrimmed merged copy of the held rows
- [`apps/desktop/src/chat/chat-thread.tsx:250`](../../../apps/desktop/src/chat/chat-thread.tsx#L250): older history is an explicit 'Load older messages' button
- [`apps/desktop/src/screens/chat-screen.tsx:342`](../../../apps/desktop/src/screens/chat-screen.tsx#L342): ChatThread is keyed per channel and remounts pinned to the bottom; useChatViewport keeps no per-channel scroll memory

**Recommendation**

In conversation-model.ts, trim instead of throwing. When appending at the tail, drop rows from the oldest end and set `before` to the oldest kept sequence. When prepending an older page, drop rows from the newest end and mark the window detached, sharing that flag with chat-ux-open-at-first-unread, so that 'Jump to latest' reloads the tail. Prune the verification map with the rows, and apply the same trim to useChatHistory's latest.current copy. Set the per-channel trim target well below the shared cache budget (for example 400 rows or 3 MiB), or give the open channel a budget separate from the background LRU entries, so one long scrollback does not evict every other channel. Replace the button with an IntersectionObserver sentinel at the top that calls load(before), and keep the button as an accessible fallback. Store {anchorMessageId, offset} per (storeId, channel) in a small map owned by the history cache. Restore it in useChatViewport's layout effect when the channel remounts, and fall back to the anchored-unread rule when the anchor has been trimmed. Rewrite tests/chat-history-cache.test.ts:179 to assert that tail growth past the cap keeps the newest N rows, and add a test that paging older past the cap keeps a contiguous window.

**Already tracked:** book/20-desktop.qmd 'History paging' documents the 16-channel / 1,000-row / 8 MiB cache bounds, but not the error and reset behaviour at the cap

**Mockup:** [Open at first unread](../mockups/chat-unread-anchored-open.html)

<details><summary>Verifier note</summary>

The core claims hold. conversationResult throws code 'chat-limit' with the quoted message when the window exceeds 1,000 rows or 8 MiB (conversation-model.ts:90-98). ChatHistoryCache.accept catches it only when before === null, and then rebuilds the window from the incoming page alone (history-cache.ts:156-171). Older pages rethrow, and failureAlert shows the error as crit. The test 'a full retained window can advance to the newest page...' (tests/chat-history-cache.test.ts:179) codifies this: after 1,000 rows plus one incoming row, the window holds only ['1001']. With incremental `after` reads the incoming page can be a single row, so most of the scrollback is lost. Older history loads only through the 'Load older messages' button. ChatThread is keyed `${channel.id}:${channel.readable}` (chat-screen.tsx:342) and remounts with followBottom=true, so scroll position is not restored. The book documents only the cache bounds. The finding misses one thing that changes the recommendation. The cache row and byte limits are a single budget across all 16 channel entries (history-cache.ts:187-203), so one channel holding 1,000 rows evicts every other channel's window. The summary's 'the LRU cache keeps its messages' is therefore conditional, and a per-channel trim target of 1,000 rows would use the whole cache. useChatHistory also keeps its own untrimmed copy in latest.current (use-chat-history.ts:156-167), which would need the same trim.

</details>

### chat-ux-keyboard-a11y

**Keyboard navigation and screen-reader support for the conversation**

- Type: feature-refinement
- Priority: medium
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

Ctrl+K reaches channels through the search palette, but there is no Alt+Up/Down to move between channels, no Alt+Shift+Up/Down to jump to the next unread channel, and no Esc to mark a channel read. Messages are not focusable, and the only message action and the sidebar's channel menu open only on right-click, so keyboard users cannot reach them. The message list is a plain div with aria-label and aria-busy, not a log or live region, so screen readers do not announce new incoming messages.

**Evidence**

- [`apps/desktop/src/chat/chat-thread.tsx:241`](../../../apps/desktop/src/chat/chat-thread.tsx#L241): message scroller is a div with aria-label='Message history' and aria-busy; no role='log' or aria-live; articles are not focusable
- [`apps/desktop/src/chat/message-text.tsx:184`](../../../apps/desktop/src/chat/message-text.tsx#L184): Copy message opens only via onContextMenu on non-focusable text
- [`apps/desktop/src/screens/chat-teams.tsx:451`](../../../apps/desktop/src/screens/chat-teams.tsx#L451): column menu via onContextMenu on the aside; rows are buttons (924), so keyboard contextmenu works only where the webview dispatches it
- [`apps/desktop/src/chat/use-chat-read-intent.ts:43`](../../../apps/desktop/src/chat/use-chat-read-intent.ts#L43): reads are auto-marked when the focused thread is at the bottom, which limits what Esc-to-mark-read adds
- [`apps/desktop/src/shell/back-inputs.ts:19`](../../../apps/desktop/src/shell/back-inputs.ts#L19): Alt+Left/Right is already back/forward; Alt+Up/Down is free
- [`apps/desktop/src/shell/sidebar.tsx:721`](../../../apps/desktop/src/shell/sidebar.tsx#L721): Ctrl+Tab rail cycling, guarded by anyDialogOpen(), is the pattern to follow

**Recommendation**

Desktop-UI only. (1) In ChatTab, add document key handlers that do nothing while anyDialogOpen() is true. Alt+Up/Down moves to the previous/next listed channel, and Alt+Shift+Up/Down to the next unread, unmuted channel. Leave Alt+Up/Down to the textarea when the caret is not on the first or last line. (2) Give message rows a roving tabindex: Up/Down/Home/End to move, Enter, Shift+F10 or the ContextMenu key to open the message actions, and Esc back to the composer. (3) Add a visually hidden role='status' aria-live='polite' announcer for messages that arrive while the window is focused, rate-limited, naming sender and channel only (no body, consistent with the notification previews setting). (4) For channel rows, open the existing column menu at the row's getBoundingClientRect() when the contextmenu event comes from the keyboard, rather than at clientX/clientY. (5) Optionally, Esc in a scrolled-up thread with an empty composer marks the channel read through the newest message. (6) Add a '?' shortcut sheet that lists only bindings that exist, in the platform's modifier notation. Cover the bindings with keyboard tests in chat.render.test.tsx and chat-teams.render.test.tsx.

**Mockup:** [Open at first unread](../mockups/chat-unread-anchored-open.html), [Message reading and actions](../mockups/chat-reading-and-message-actions.html)

<details><summary>Verifier note</summary>

Core claim holds. The chat code has no Alt+arrow channel navigation, no next-unread binding and no Escape binding in the thread. Message articles are not focusable. The 'Copy message' menu opens only on contextmenu over non-focusable text (message-text.tsx:184). The scroller is a plain div with aria-label and aria-busy and no log role or live region (chat-thread.tsx:241-249), so incoming messages are not announced. Four corrections: (1) Sidebar channel rows are <button>s (chat-teams.tsx:924). In Chromium-based webviews, the ContextMenu key and Shift+F10 already fire a contextmenu event that bubbles to the aside's handler (chat-teams.tsx:451). The gap is platform-dependent: WKWebView on macOS has no such key. Keyboard-fired events also carry unreliable clientX/clientY. (2) Every live item in the sidebar menu (Open channel, Channel info, Go to team, Add people) is also reachable through other controls, so reaching that menu by keyboard matters less than the claim implies. (3) Reads are already marked automatically when the focused thread is at the bottom (use-chat-read-intent.ts:43-60), so Esc-to-mark-read only helps when the thread is scrolled up. (4) The mockup brief lists bindings that neither exist nor appear in the recommendation ('↑ in empty composer: edit last unsent', 'Ctrl+B/I/E: bold/italic/code'). Alt+Left/Right is already used for back/forward (back-inputs.ts:19-22). Alt+Up/Down is free, but on macOS it moves the caret to a paragraph boundary inside the multi-line composer, and the app's other shortcuts accept ⌘ or Ctrl.

</details>

### chat-ux-link-label-spoofing

**Markdown link labels can disguise the destination; show the host and confirm mismatches**

- Type: security
- Priority: medium
- Effort: S
- Layers: desktop-ui, desktop-native
- Verification: adjusted

A message such as `[https://bank.example/login](https://evil.example/)` renders as `https://bank.example/login`, and clicking it opens evil.example through the host's open_chat_link with no confirmation and no title or status-bar hint of the real destination. Chat content comes from any team member, and a team member's device may be compromised. In an app whose purpose is protecting secrets, a label-versus-destination mismatch is a credible phishing vector. The scheme and credential checks in safeChatLink and safe_external_url do not address it.

**Evidence**

- [`apps/desktop/src/chat/message-text.tsx:102`](../../../apps/desktop/src/chat/message-text.tsx#L102): `[label](url)` renders token.slice(1, split) as the visible label with the URL hidden; the Link component (56-83) sets no title
- [`apps/desktop/src-tauri/src/commands/chat.rs:340`](../../../apps/desktop/src-tauri/src/commands/chat.rs#L340): open_chat_link opens any http(s) URL via open/xdg-open after validation; no confirmation step

**Recommendation**

Desktop-UI: render every link with title={url}. When the label differs from the URL, append a muted host suffix ('(evil.example)') after the link, taken from new URL(url).hostname, which is already punycode. If the label parses as a URL or bare hostname whose host differs from the destination host, style the link as a warning. Clicking it then opens a renderer confirmation naming both hosts, with Copy link, Cancel (default) and Open anyway. Do not add a renderer-controlled `confirm` flag to open_chat_link. It cannot enforce anything, and the host never sees the label. If a host-enforced step is wanted, open_chat_link must show a native dialog naming the destination host for every chat link, without a flag. Tests: label/host mismatch; punycode host shown as xn--; label identical to URL (no suffix); relative-looking labels such as 'here' (suffix only, no warning).

**Mockup:** [Message reading and actions](../mockups/chat-reading-and-message-actions.html)

<details><summary>Verifier note</summary>

The core claim holds. In message-text.tsx:102-111, `[label](url)` renders token.slice(1, split) as the visible text. The Link component (56-83) sets no title and calls openChatLink directly. open_chat_link (chat.rs:340) validates only scheme, host, credentials and control characters (safe_external_url, 375), then runs open/xdg-open with no confirmation. No spoofing or mismatch handling exists in the TS, the host, ISSUES.md or the book. One part of the recommendation is unsound. A renderer-supplied `confirm: bool` cannot stop the renderer from skipping the confirmation, because the renderer chooses the flag. The host also cannot detect a label/host mismatch, because only the URL crosses IPC. A host-side dialog would only help if it ran on every link without a flag. Also, `new URL(...).hostname`, and therefore safeChatLink's returned href, already gives the punycode form, so showing the xn-- host needs no extra decoding.

</details>

### chat-ux-local-mute

**Make mute real: a device-local per-channel and per-team mute that drives badges, plus a 'Mentions only' alert mode**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: desktop-native, desktop-ui, agent
- Verification: adjusted

The UI reads conversation.muted and .hidden everywhere: captions, dimmed rows, and exclusion from team and rail unread totals. But this server hard-codes both to false, and the protocol has no method to set them, so 'Muted' can never appear. Separately, the host stores a per-channel alert override (inherit / all / none) in chat_local.rs. Setting alerts to 'None' still leaves the channel's unread count in the team badge and the rail's Chat badge, so there are two half-features. There is no team-level default and no 'Mentions only' mode.

**Evidence**

- [`crates/foks-rpc/src/realtime.rs:13`](../../../crates/foks-rpc/src/realtime.rs#L13): RealtimeRequest variants: ChatCapabilities, CreateChannel, ListChannels, Send, GetThread, GetInboxVersion, GetChangedThreads, ReadThrough, PollInbox, SelectVhost, Recents; no mute/hide setter
- [`crates/foks-server-db/src/realtime/inbox.rs:535`](../../../crates/foks-server-db/src/realtime/inbox.rs#L535): RtInboxChannel { hidden: false, muted: false } always
- [`apps/desktop/src/chat/unread.ts:11`](../../../apps/desktop/src/chat/unread.ts#L11): teamUnread excludes c.hidden || c.muted from counts
- [`apps/desktop/src-tauri/src/commands/chat_local.rs:24`](../../../apps/desktop/src-tauri/src/commands/chat_local.rs#L24): overrides: BTreeMap<String,bool>, bounded at 4096 entries (lines 149, 324)
- [`apps/desktop/src/chat/notification-provider.tsx:252`](../../../apps/desktop/src/chat/notification-provider.tsx#L252): alert mode inherit/all/none derived from overrides; unread badges ignore it
- [`crates/foks-agent/src/chat.rs:722`](../../../crates/foks-agent/src/chat.rs#L722): notification-history text truncated to 256 characters
- [`ISSUES.md:187`](../../../ISSUES.md#L187): mentions are listed as deferred extended-chat features

**Recommendation**

Device-local first, with no protocol change. Extend the chat_local Settings with `muted: BTreeSet<String>`, keyed by the same notificationKey(scope, channel) derivation and bounded at 4096 like overrides. Add a per-team default alert mode. Expose both in the local Session. In inbox-service's published projection, set muted = server.muted || local.muted, so channelMeta, teamUnread, channelUnreadTotal and the rail badge work unchanged. Muting implies alerts 'none' unless the user chooses otherwise. An optional 'Mentions only' mode should be labelled as a text match. It matches @<own username> case-insensitively at word boundaries within the first 256 characters the notification consumer reads. Name it as distinct from the protocol mentions deferred in ISSUES.md:187-188. Add a UI item 'Mute channel' to the channel context menu and info panel, and a team-level alerts default to the team header menu. Later step (agent): sync these preferences as an encrypted record in the user's own KV store, so the server holds only ciphertext. Record in ISSUES.md that server-side muted/hidden cannot be set on the pinned protocol.

**Mockup:** [Chat channel column](../mockups/chat-channel-column.html), [Channel readers and alerts](../mockups/chat-channel-info-and-alerts.html)

<details><summary>Verifier note</summary>

The core claim holds. Server inbox.rs:535-536 always returns hidden: false and muted: false. The realtime request enum (foks-rpc/src/realtime.rs:13-25: ChatCapabilities, CreateChannel, ListChannels, Send, GetThread, GetInboxVersion, GetChangedThreads, ReadThrough, PollInbox, SelectVhost, Recents) has no setter. The cited foks-proto realtime.rs:561 is the argument structs, not the method set. teamUnread (unread.ts:11-14) excludes hidden and muted conversations. chat_local.rs:21-25 holds `overrides: BTreeMap<String,bool>`, bounded at 4096 (149, 324). notification-provider.tsx:251-252 maps overrides to inherit/all/none, and no badge code reads overrides. The agent truncates notification text to 256 characters (agent chat.rs:722-726). Contrary to the empty already_tracked field, ISSUES.md:187-188 lists 'mentions' among the deferred extended-chat features. A 'Mentions only' mode built on matching '@username' in text is a separate local heuristic. It must be presented as one and must not be described as protocol mentions. The device-local mute does not depend on it.

</details>

### chat-ux-message-actions

**Add a hover/focus action bar per message: copy, quote-reply, copy link, details (no protocol change)**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

The only message action is 'Copy message', in a context menu opened by right-clicking a non-focusable div, so keyboard users cannot reach it at all. Reply, react, edit and delete are absent. The pinned protocol defines Edit/Delete/Reaction/Reply message types, but the server accepts only Basic sends and the client renders the other types as 'not supported yet', and ISSUES.md defers extended chat. Several useful actions need no protocol change: a quote-reply written as Basic text using the blockquote syntax the renderer already supports, an in-app permalink to a message sequence, and a details popover with sent and received times and verification status.

**Evidence**

- [`apps/desktop/src/chat/message-text.tsx:182`](../../../apps/desktop/src/chat/message-text.tsx#L182): onContextMenu on a non-focusable div is the only entry point; the menu holds one item, 'Copy message' (202-215)
- [`crates/foks-server-db/src/realtime/messages.rs:16`](../../../crates/foks-server-db/src/realtime/messages.rs#L16): the server rejects any send whose kind is not RtMessageType::Basic
- [`crates/foks-proto/src/realtime.rs:205`](../../../crates/foks-proto/src/realtime.rs#L205): RtMessageType reserves Edit=2, Delete=3, Reaction=4, Reply=6
- [`crates/foks-client/src/realtime/history.rs:299`](../../../crates/foks-client/src/realtime/history.rs#L299): non-Basic kinds become ChatContent::Unsupported
- [`apps/desktop/src/chat/chat-thread.tsx:349`](../../../apps/desktop/src/chat/chat-thread.tsx#L349): send_time and sequence are reachable only through the time-element tooltip
- [`apps/desktop/src/chat/message-text.tsx:173`](../../../apps/desktop/src/chat/message-text.tsx#L173): '> ' blockquotes are rendered, so a quote-reply can be plain Basic text
- [`apps/desktop/src-tauri/src/commands/vault.rs:209`](../../../apps/desktop/src-tauri/src/commands/vault.rs#L209): store ids are local JSON (profile, accountAlias, teamAlias, teamId), so a ?store= link does not carry over to other users
- [`apps/desktop/src/chat/message-text.tsx:9`](../../../apps/desktop/src/chat/message-text.tsx#L9): safeChatLink accepts only absolute http(s) URLs, and openChatLink opens them externally with xdg-open/open, so an in-app link pasted in chat would not navigate
- [`apps/desktop/src/chat/conversation-events.ts:59`](../../../apps/desktop/src/chat/conversation-events.ts#L59): verification holds missing_predecessors.length > 0 for the page, not a per-message chain result

**Recommendation**

Desktop-UI only. Add a MessageActions component rendered on hover and on focus-within of each article.chat-message, and make rows focusable (see chat-ux-keyboard-a11y). Actions: (1) Copy text. (2) Quote reply: insert '> **@author** · 9:14\n> first lines…\n\n' at the composer caret through service.setDraft, then focus the composer. (3) Details popover: author, server receive time (insert_time), sender clock (send_time), sequence number, and the page-level verification state, labelled for example 'Earlier messages on this page could not be checked', not as a per-message chain result. Move 'Copy message' into the same component; outgoing rows already render MessageText. Do not show disabled React/Edit/Delete buttons. Treat 'Copy link' as a separate, later item. It needs (a) a portable identifier (team id hex + channel id + sequence) resolved against the viewer's own stores, not the local StoreRef, (b) MessageText recognising that form and routing it through in-app navigation instead of safeChatLink/openChatLink, and (c) optionally an OS-registered URI scheme. On open it would load an anchored window around the sequence and highlight the row.

**Already tracked:** ISSUES.md 'Existing disclosed limitations' (edits, deletion, reactions, threads, mentions and attachments remain disabled) and book/16-chat.qmd 'Current limits'; this adds actions that need no protocol change as a first step

**Mockup:** [Message reading and actions](../mockups/chat-reading-and-message-actions.html)

<details><summary>Verifier note</summary>

Most of the finding holds. The only message action is 'Copy message' in a ContextMenu opened by onContextMenu on a non-focusable div (message-text.tsx:182-215). The server rejects non-Basic sends (messages.rs:16). RtMessageType reserves Edit=2, Delete=3, Reaction=4 and Reply=6 (realtime.rs:205). Non-Basic messages become ChatContent::Unsupported (history.rs:299). Sequence and send_time appear only in the time-element tooltip (chat-thread.tsx:349). Blockquotes are rendered (message-text.tsx:173). The tracking claims (ISSUES.md 'Existing disclosed limitations', book 16-chat 'Current limits') are accurate. 'Keyboard users cannot reach it at all' is slightly overstated: the context-menu key on a focused link or a 'Copy code' button inside the message bubbles to the div. The 'Copy link' action cannot work as specified. (a) Store refs are local JSON {profile, accountAlias, teamAlias, teamId} (src-tauri/src/commands/vault.rs:209), so a link built from production-codec's `store` parameter means nothing to another team member. (b) No URI scheme is registered for deep links. safeChatLink accepts only http(s), and openChatLink shells out to xdg-open/open, so a pasted link would be neither clickable nor routed back into the app. The mockup's 'paste the link into another channel and click it' state therefore needs new infrastructure. Also, HistoryWindow.verification holds the page-level missing_predecessors flag (conversation-events.ts:59), not a per-message chain result, so 'Chain checked' per message overstates it. The priority is lowered because the actions that need no protocol change are refinements; the keyboard-access gap is covered in chat-ux-keyboard-a11y.

</details>

### chat-ux-message-rendering

**Message rendering: autolink bare URLs, keep paragraph breaks, style code blocks, highlight @mentions**

- Type: feature-refinement
- Priority: medium
- Effort: S
- Layers: desktop-ui
- Verification: confirmed

The renderer links only `[label](url)` syntax, so the most common case, a pasted bare https URL, stays plain text (confirmed in the mock build). Blank lines are dropped (`else if (line)`), which collapses paragraphs into adjacent lines. Code fences render as unstyled monospace with a permanently visible 'Copy code' button and a status span. List bullets hang outside the text column. There is no visual treatment for @username, so later mention counting has nothing to build on.

**Evidence**

- [`apps/desktop/src/chat/message-text.tsx:88`](../../../apps/desktop/src/chat/message-text.tsx#L88): inline pattern handles only `code`, **, *, `[label](url)`; bare https://… is not matched
- [`apps/desktop/src/chat/message-text.tsx:177`](../../../apps/desktop/src/chat/message-text.tsx#L177): `else if (line)` skips empty lines, so paragraph spacing is lost
- [`apps/desktop/src/chat/message-text.tsx:150`](../../../apps/desktop/src/chat/message-text.tsx#L150): each code block always renders a 'Copy code' Button and a role=status span
- [`apps/desktop/src/screens/chat.css:1260`](../../../apps/desktop/src/screens/chat.css#L1260): .chat-message-text pre has no background, padding or max-height

**Recommendation**

Desktop-UI only, inside message-text.tsx and keeping its bounded, non-recursive design. (1) Extend the inline pattern with a bare-URL alternative (https?://[^\s<>]+ with trailing-punctuation trimming) passed through safeChatLink. (2) Emit an empty-line spacer (or margin on the following block) for blank lines. (3) Style pre with a surface background, 8px padding, max-height 360px with overflow, and a copy button shown on hover/focus-within only. (4) Tokenize @name, and when the name is in partyNames give it a 'mention' class; give the current user's own username a highlighted 'mention self' class. This is text styling only, with no protocol semantics. (5) Indent lists inside the body column. Add cases to tests/chat-text.test.tsx for bare URLs with trailing ')' or '.', blank-line preservation, and @mention tokens.

**Already tracked:** Mentions as a protocol feature are deferred (ISSUES.md 'Existing disclosed limitations'); the @-highlight here is only styling of Basic text

**Mockup:** [Message reading and actions](../mockups/chat-reading-and-message-actions.html)

<details><summary>Verifier note</summary>

Each claim was checked against the code. The inline pattern (message-text.tsx:88-89) matches only `code`, **bold**, *italic* and `[label](url)`, so bare https URLs stay plain text. `else if (line)` at line 177 skips blank lines. Every block is a <p> with `margin: 2px 0 0` (chat.css:1014-1021), so 'a\nb' and 'a\n\nb' render the same. Each fenced block always renders a 'Copy code' Button plus a role=status span (Copy component, lines 37-52 and 150-156). `.chat-message-text pre` (chat.css:1260-1264) sets only max-width, overflow-x and white-space, with no background, padding or max-height. The universal reset `*{margin:0;padding:0}` (shell.css:2) removes list padding, so outside-positioned bullets hang left of the text column. No document records autolinking or the Markdown subset as a deliberate omission, and mentions are deferred only as a protocol feature. The recommendation fits the bounded, non-recursive renderer: bare URLs can pass through the existing safeChatLink, and @-highlighting is styling of Basic text.

</details>

### chat-ux-sender-identity

**Resolve every sender to a verified username, including departed members, and mark former members**

- Type: feature-refinement
- Priority: medium
- Effort: M
- Layers: agent, client-lib, desktop-ui
- Verification: adjusted

Sender names come only from the current team roster. A message from someone who has left, or any message while the roster has failed or not yet loaded, shows the author as a truncated hex party ID like '01abcdef01…1234'. That ID is meaningless to users and still feeds the avatar hue and initial. Production messages always have a sender, because foks-client rejects a missing one. The 'Team member' label in the current screenshots comes only from the mock's null sender. Previews in the column, if restored, would have the same problem.

**Evidence**

- [`apps/desktop/src/chat/presentation.ts:115`](../../../apps/desktop/src/chat/presentation.ts#L115): partyNames maps only partiesOf(snapshot, store) users; no fallback source
- [`apps/desktop/src/chat/chat-thread.tsx:300`](../../../apps/desktop/src/chat/chat-thread.tsx#L300): avatarName = senderNames.get(sender) ?? shortId(sender)
- [`apps/desktop/src/screens/chat-screen.tsx:146`](../../../apps/desktop/src/screens/chat-screen.tsx#L146): senderNames built from the roster; only the actor is patched in from the account list
- [`crates/foks-client/src/realtime/history.rs:291`](../../../crates/foks-client/src/realtime/history.rs#L291): a message without a sender is a ChatChannelIntegrity error, so sender is always present in production
- [`apps/desktop/src/chat-mock.ts:77`](../../../apps/desktop/src/chat-mock.ts#L77): mock seeds sender: null, which produces the 'Team member' label seen in the screenshot

**Recommendation**

Agent/IPC: add `senders: [{uid, username: Option, member: bool}]` to ChatResult::History in agent-proto. In chat-contract.ts, validate at most CHAT_PAGE_ROWS distinct entries, user-entity uid hex ('01' prefix, 66 chars) and username ≤ CHAT_LABEL_BYTES. In the agent, take `member` from the team chain the chat session already verifies (team.verified.members()). Resolve usernames for the page's distinct senders with load_and_pin_user_as_local_team using the team view token, which this server still authorizes for removed members. Confirm the same against Go v0.1.9. Fall back to locally pinned user state, and cache results in a bounded per-host LRU in the agent so pages do not trigger repeated chain loads. A failed load yields username: None, not an error. Desktop: prefer the roster name, then the history-provided name with a 'former member' marker when member=false, then a neutral 'Unknown member' with the full UID in a tooltip and a Copy ID action, never raw hex as the name. Keep avatar hue keyed on the UID.

**Mockup:** [Delivery and integrity](../mockups/chat-delivery-and-integrity.html)

<details><summary>Verifier note</summary>

The core claim holds. partyNames (presentation.ts:114-127) draws only from the snapshot roster. chat-thread.tsx:299-301 falls back to shortId(sender), which gives '0123456789…abcd' (format.ts:60). chat-screen.tsx:145-151 patches in only the actor. history.rs:291-294 rejects a missing sender. chat-mock.ts:77 seeds `sender: null`, which produces 'Team member'. Rosters come from listGroupDetails in snapshot-projection.ts:669, so a failed or absent roster leaves every sender as hex. Former members are never on the roster, which uses list_team_members over the current verified members. The recommendation misstates what the agent already loads. The chat session (client-app chat.rs with_chat/with_chat_read) loads the team chain, which gives current membership, but not member user chains. Usernames come only from the expand_roster path of ListTeamMembers (client-app team.rs:1271-1285, load_and_pin_user_as_local_team). So the agent needs a per-sender user-chain load. On this server that load still works for removed members: authorize_user_chain_load checks team_local_view_permissions (server user.rs:384-405), and those rows are inserted (server-db team.rs:336, team_invitations.rs:224) but never deleted. Go v0.1.9 should be checked for the same behaviour. The fallback should not assume the load succeeds.

</details>

### chat-ux-thread-decomposition

**Split ChatThread into header, list, row and composer, and derive display rows as a pure model**

- Type: maintainability
- Priority: medium
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

ChatThread takes 24 props, and its single JSX tree (about 316 lines of a 458-line function) renders the header, eight alert sources, the history edge, day and NEW separators, grouping, three row structures (incoming, OutgoingRow, tracked operations), the jump button and the composer with its key handling. The data hooks are already extracted, but the rows' presentation is not. Incoming and outgoing rows duplicate the avatar, header and time markup with different time formatters (messageClock vs relativeMessageTime). The pending refinements (focusable rows, delivery state, audience header) would all land in this one component. Leftovers include the unused previewLine and roleTextWithoutBand, and inline import() prop types.

**Evidence**

- [`apps/desktop/src/chat/chat-thread.tsx:26`](../../../apps/desktop/src/chat/chat-thread.tsx#L26): 24 destructured props (26-51) with their type annotation through line 96; the function ends at line 483
- [`apps/desktop/src/chat/chat-thread.tsx:292`](../../../apps/desktop/src/chat/chat-thread.tsx#L292): inline per-row derivation of isNew, author, day separator and grouping inside the JSX map
- [`apps/desktop/src/chat/outgoing-row.tsx:56`](../../../apps/desktop/src/chat/outgoing-row.tsx#L56): separate article/avatar/header structure duplicating chat-thread.tsx:331-353, with relativeMessageTime instead of messageClock
- [`apps/desktop/src/chat/chat-thread.tsx:56`](../../../apps/desktop/src/chat/chat-thread.tsx#L56): inline import() types at lines 56, 75, 77, 92 and 485
- [`apps/desktop/src/chat/presentation.ts:221`](../../../apps/desktop/src/chat/presentation.ts#L221): previewLine unused; roleTextWithoutBand (208) also unused
- [`apps/desktop/tests/chat.render.test.tsx:266`](../../../apps/desktop/tests/chat.render.test.tsx#L266): asserts that channel rows carry no preview line, so reusing previewLine there would reverse a tested decision

**Recommendation**

Do this before the UI refinements. (1) Extract a pure threadRows(messages, outgoing, operations, {actor, newFrom, names}) that returns a discriminated list of day, new and message rows, with author and grouping resolved, and unit-test it beside tests/chat-message-grouping.test.ts. (2) Split the presentation into ThreadHeader, MessageList (owning useChatViewport, read intent, history edge and jump button), MessageRow (shared by incoming and outgoing rows, with a delivery/status slot) and Composer. (3) Move request, markRead, acceptHistory and the sends service into a conversation context provider, like send-provider and inbox-provider, instead of threading props from chat-screen.tsx. (4) Use one absolute time formatter for all rows, with relative age only in tooltips. (5) Delete previewLine (channel rows intentionally show no preview) and either delete roleTextWithoutBand or use it for the channel-audience labels. Replace the inline import() types with top-level type imports.

<details><summary>Verifier note</summary>

Core claim holds. ChatThread is one large component that renders the header, eight alert sources (alerts, failure, readError, missing, sendError, loadError, cleanupError, queueFull), the history edge, day and NEW separators, grouping, three row structures, the jump button and the composer. OutgoingRow duplicates the avatar, header and time markup and uses relativeMessageTime where incoming rows use messageClock (outgoing-row.tsx:56-83 vs chat-thread.tsx:331-353). Inline import() types appear at lines 56, 75, 77, 92 and 485, and previewLine is unused. Corrections: (1) ChatThread takes 24 props, not 25. (2) The function spans lines 26-483 (about 458 lines) and its JSX lines 167-482 (about 316), not a '480-line render'. (3) The state logic is already split into hooks (useMessageComposer, useChatViewport, useChatHistory, useChatReadIntent, useComposerSize), so the remaining work is mostly presentational. (4) 'Use previewLine for the channel-list previews' contradicts a deliberate decision: chat.render.test.tsx:266-277 asserts that channel rows carry no preview line. previewLine should be deleted, and roleTextWithoutBand is unused as well. The chat code already uses React context providers (send-provider, inbox-provider), so a conversation context fits the existing pattern.

</details>

### chat-ux-mock-fidelity

**Give the chat mock realistic scenarios so design review and screenshots reflect production**

- Type: tooling
- Priority: low
- Effort: S
- Layers: tooling, desktop-ui
- Verification: adjusted

The VITE_FOKS_MOCK chat seeds one unnamed channel (shown as #general) with one message whose sender is null, so mock screenshots show a 'Team member' author that production history cannot produce, because the client rejects messages without a sender. The mock cannot show author grouping, departed senders, long unread runs, muted or hidden channels, unsent or rejected operations, missing predecessors, a degraded inbox or stopped channels. Render tests reach these states by rewriting mock replies test by test, so design review of the planned refinements in the mock build has no realistic fixtures.

**Evidence**

- [`apps/desktop/src/chat-mock.ts:55`](../../../apps/desktop/src/chat-mock.ts#L55): single channel id 'ab'×16, name '' (renders as #general), description 'A place for the whole team.'
- [`apps/desktop/src/chat-mock.ts:77`](../../../apps/desktop/src/chat-mock.ts#L77): seed message sender: null, which renders as 'Team member'
- [`apps/desktop/src/chat-mock.ts:209`](../../../apps/desktop/src/chat-mock.ts#L209): missing_predecessors always []; degraded false (225); hidden/muted false (242-243)
- [`crates/foks-client/src/realtime/history.rs:291`](../../../crates/foks-client/src/realtime/history.rs#L291): a history message without a sender is rejected as an integrity error, so production threads never show a null sender
- [`apps/desktop/src/mock-bridge.ts:44`](../../../apps/desktop/src/mock-bridge.ts#L44): existing mock-only `?state=agent-lost` switch, the place for a chat scenario parameter
- [`apps/desktop/tests/chat.render.test.tsx:1228`](../../../apps/desktop/tests/chat.render.test.tsx#L1228): render tests already inject missing_predecessors and uncertain operations by rewriting mock replies

**Recommendation**

Add a mock-only scenario switch in mock-bridge.ts/chat-mock.ts, read from the URL the way `?state=agent-lost` already is, and leave production-codec.ts untouched. Provide deterministic fixtures. busy: Engineering with #general (120 messages, read_through 78), #deploys, an admin-tier #incidents and #random, with senders from the fixture roster plus one UID absent from it. failures: operations in prepared and uncertain states and with the rejection codes the UI maps, plus one intent load error. integrity: a page with missing_predecessors, degraded: true, one blocked channel, and a muted and a hidden conversation. Give the seed message the fixture owner's UID as sender. Export the fixtures so chat.render.test.tsx can replace its ad-hoc reply rewriting with named scenarios.

<details><summary>Verifier note</summary>

Core claim holds. mockChat seeds one unnamed channel, which renders as #general (chat-mock.ts:55-67, channelLabel falls back to 'general'), with one message whose sender is null (77). It hardcodes missing_predecessors [] (209), degraded false (225) and hidden/muted false (242-243). Production history cannot yield a null sender: foks-client/src/realtime/history.rs:291-294 rejects a message without a sender as an integrity error. The production codec takes only store and channel for chat. Corrections: (1) 'Cannot be covered by render tests' is overstated. chat.render.test.tsx loads mockBridge, and through it mockChat, and already injects these states by rewriting replies: missing_predecessors at 1228, uncertain operations at 1285-1313 and 1575 onward, null-sender rows at 1158-1176. chat-teams.render.test.tsx builds muted, hidden and admin channels (401, 1033-1042). The real gap is design review and screenshots in the mock build, plus duplicated per-test reply rewriting. (2) mock-bridge.ts:44-48 already uses a mock-only `?state=agent-lost` switch. A scenario parameter belongs in the mock bridge, not in navigation/production-codec.ts. As tooling, the priority is lower than medium.

</details>
