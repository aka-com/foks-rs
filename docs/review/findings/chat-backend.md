# Chat protocol and backend stack

Area key `chat-backend`. 13 verified findings. Part of the [foks-rs review](../README.md).

## Current state

Basic chat is a complete, conservative implementation of the pinned Go v0.1.9 realtime protocol. It covers the 11 upstream methods except rtGetChannel (crates/foks-server/src/rpc/generated/routes.rs:1981-1995), plus one Rust extension (foksChatCapabilities at position 65536, which always answers basic_only). Strong parts: per-channel anchor ledger (history.rs, repositories/chat.rs), journaled prepare/attempt/reconcile sends with submission MACs, a monotonic per-user inbox version with pull-based long polling, and dirty-then-reconcile membership invalidation. The Rust-only format-2 substrate (chat_context.rs, chat_v2.rs) is crypto only. It has no server event store or RPC, every server read path calls require_basic, and listing filters format=1. Feature status for a desktop UI. Edits, reactions and replies: enum values plus a v0.1.9 Pegged body arm (realtime.rs:205, 268-357), but the FOKS-RS server rejects non-Basic sends, crypto refuses to open them, and history renders them as Unsupported. Delete and attachments: enum values only, with no body arm and no format-2 attachment purpose. Channel rename, description, archive and delete, and role changes: configuration is immutable by schema, and upstream has no method for any of them. Read receipts of others: the server stores every member's read_through but exposes only the caller's own. Typing indicators: none. DMs: ad-hoc teams can be created, but chat refuses non-named teams on both client and server. Sender names: the agent returns a hex UID and the desktop maps it from the current roster. Search: none. Notification preferences: the desktop-native shell keeps device-local per-channel on/off overrides, the server always returns hidden=false and muted=false, and no setter exists anywhere. Mentions: none. The highest-value issues: one malformed message from any channel writer disables reading and sending in that channel. Capability negotiation fails closed on any version skew, so it cannot carry the features it was built to advertise. Send-side capacity refusals become permanent uncertain operations. Revoking a member's access degrades their inbox on the Rust server itself. The minimal path for most UI features is either a client-side change, Go-compatible (Pegged bodies, rtGetChannel, personal-KV-synced preferences, KV attachments, local search), or a capability-gated Rust-only server method whose results keep Go wire shapes. The capability fix has to land first.

## Index

| Finding | Type | Priority | Effort |
|---|---|---|---|
| [One malformed message from any channel writer stops reading and sending in that channel](#chat-backend-single-message-poisons-channel) | security | high | M |
| [Attachments stored in the team KV under channel-scoped roles, referenced from Basic text](#chat-backend-attachments-via-team-kv) | missing-feature | medium | L |
| [Send-side capacity refusals are reported as generic 12001, leaving operations permanently Uncertain](#chat-backend-capacity-refusals-become-uncertain) | correctness-risk | medium | S |
| [Channel rename, description, posting role and archive via a Rust-only update method that keeps Go wire metadata](#chat-backend-channel-management-format1) | missing-feature | medium | L |
| [Agent-owned per-channel alert/hide preferences synced via personal KV, local mentions and sender labels](#chat-backend-notification-prefs-mentions-names) | missing-feature | medium | M |
| [Define retention as an explicit history floor; the client's gap, predecessor and recovery logic assume dense sequences](#chat-backend-retention-floor-vs-anchor-ledger) | correctness-risk | medium | M |
| [The FOKS-RS server itself puts an inbox into the degraded state on every access revocation](#chat-backend-revocation-degrades-inbox) | correctness-risk | medium | S |
| [Stage 1 quota: drop the redundant envelope column and keep retained-byte counters in the send transaction](#chat-backend-stage1-redundant-envelope-and-counters) | performance | medium | M |
| [Stage 2 compaction: time-ordered submission IDs plus a durable floor make expiry safe](#chat-backend-stage2-submission-floor) | maintainability | medium | M |
| [Chat capability discovery fails closed on version skew, forces format 2 for every feature, and is unused by the agent](#chat-backend-capability-negotiation-not-forward-compatible) | correctness-risk | low | S |
| [Every chat read lists and decrypts all channels; implement upstream rtGetChannel and reuse reads for mark-read](#chat-backend-get-channel-read-path) | performance | low | M |
| [Bounded verified history search in the agent, without server or plaintext persistence](#chat-backend-history-search) | missing-feature | low | M |
| [Typing indicators and other members' read positions as capability-gated ephemeral extensions](#chat-backend-presence-typing-receipts) | missing-feature | low | L |

### chat-backend-single-message-poisons-channel

**One malformed message from any channel writer stops reading and sending in that channel**

- Type: security
- Priority: high
- Effort: M
- Layers: client-lib, agent, protocol
- Verification: confirmed

The server cannot validate ciphertext, so any member with write access can submit a Basic message whose ciphertext does not decrypt (or decrypts to non-UTF-8, which Go treats as valid bytes). On FOKS-RS clients, that one row fails the whole history page. It also fails every later page whose first message's predecessor is that row, because the predecessor fetch calls open_message with `?`. It quarantines the channel's preview. Because prepare_fresh_send verifies the latest message first, no FOKS-RS client can send in the channel while that row is the head, and there is no deletion path. Valid v0.1.9 wire shapes the client does not support (team sender, further_user_attribution, an unavailable key generation) return non-integrity errors that abort the entire team inbox sync rather than one channel. The existing conformance test codifies whole-channel quarantine as the intended outcome.

**Evidence**

- [`crates/foks-server-db/src/realtime/messages.rs:15`](../../../crates/foks-server-db/src/realtime/messages.rs#L15): Server checks only kind, ID, ciphertext length >= 16, attribution and key role/generation; ciphertext content is opaque
- [`crates/foks-client/src/realtime/history.rs:310`](../../../crates/foks-client/src/realtime/history.rs#L310): Decryption failure and non-UTF-8 map to ChatChannelIntegrity for the whole page (lines 310-318)
- [`crates/foks-client/src/realtime/history.rs:253`](../../../crates/foks-client/src/realtime/history.rs#L253): Predecessor verification calls self.open_message(md, &m)? so a bad predecessor aborts the successor's page; unsupported predecessors are never removed from `need` and stay missing_predecessors
- [`crates/foks-client/src/realtime/history.rs:291`](../../../crates/foks-client/src/realtime/history.rs#L291): Non-user sender and further_user_attribution return ChatUnsupported errors instead of per-message content (lines 291-303)
- [`crates/foks-client/src/realtime/operations/prepare.rs:220`](../../../crates/foks-client/src/realtime/operations/prepare.rs#L220): Send preparation verifies the latest message and refuses on failure or on Unsupported content (lines 220-229)
- [`crates/foks-client/src/realtime/inbox.rs:279`](../../../crates/foks-client/src/realtime/inbox.rs#L279): Only ChatChannelIntegrity is quarantined; any other per-channel error returns Err and aborts the team sync (lines 279-289)
- [`crates/foks-server-testkit/tests/conformance/realtime.rs:1562`](../../../crates/foks-server-testkit/tests/conformance/realtime.rs#L1562): Test asserts that corrupt content blocks the whole channel
- [`tools/foks-v019-oracle/realtime_fixture_test.go:103`](../../../tools/foks-v019-oracle/realtime_fixture_test.go#L103): Go Basic body is RTMsgPlaintextBasic([]byte), so non-UTF-8 bodies are valid on the wire

**Recommendation**

Introduce per-message verdicts in foks-client history: add ChatContent::Unverifiable { reason } for decryption failure, non-UTF-8, read-role mismatch, unsupported sender or attribution, and missing key generation. Keep page-fatal errors only for server ordering or mapping contradictions: duplicate IDs or sequences, non-monotonic pages, anchor-ledger conflicts, and a predecessor-ID mismatch asserted by an authenticated message. Record every row as observed, as now, so substitution remains detectable through digests. Satisfy a predecessor check with the fetched row's (sequence, id) regardless of its content, because the claim being checked is the authenticated successor's pointer. In prepare_fresh_send, chain to the latest observed row of any verdict; the server only checks the (previous_sequence, previous_id) mapping. In hydrate_previews, map every per-channel content error to blocked_channels. Add an `unverifiable` kind to foks-agent-proto ChatContent so the desktop can render a placeholder row. Tests: on the testkit server, a member submits a raw RtSend with random ciphertext, and a second member can still page history across it, see previews, and send. This change also unblocks rolling out any new message kind (see the Pegged finding) without leaving older FOKS-RS clients unable to send.

**Already tracked:** SECURITY.md:441-447 and book/16-chat.qmd describe channel quarantine as the design; neither ISSUES.md nor the book treats a member-authored malformed message as an availability risk.

**Mockup:** [Delivery, sender identity and integrity states](../mockups/chat-delivery-and-integrity.html), [Extended chat preview: replies, files, channel settings, presence](../mockups/chat-capability-gated-extensions.html)

<details><summary>Verifier note</summary>

Every cited path holds. rt_send (messages.rs:16-22) checks only kind, ID, a ciphertext of at least 16 bytes, attribution and the predecessor shape, then the key role and generation; it never inspects ciphertext content. verify_page calls self.open_message(md,&m)? for each row (history.rs:189) and for each fetched predecessor (history.rs:253). open_message maps decryption failure and non-UTF-8 to ChatChannelIntegrity (history.rs:316,318), and maps a non-user sender or further_user_attribution to ChatUnsupported (history.rs:296,302). An unavailable key generation returns ChatKeyUnavailable from session.rs:189-199. hydrate_previews quarantines only ChatChannelIntegrity (inbox.rs:279). Every other non-network error returns Err (inbox.rs:289), and sync_inbox_gated propagates it with ? at inbox.rs:216. prepare_fresh_send runs verify_page on the latest row (prepare.rs:219) and refuses Unsupported content (prepare.rs:227). That row is never anchored, so the next row's predecessor fetch also fails, and nothing deletes from rt_messages. The conformance test at realtime.rs:1562 asserts blocked_channels==[bad]; it simulates the corruption in the transport, but the client-side effect is the same. The Go fixture builds RTMsgPlaintextBasic from []byte (realtime_fixture_test.go:103). The Go v0.1.9 server does not reject further_user_attribution or non-Basic kinds (server/realtime/messages.go stores Md.Typ unchecked), so these shapes can reach FOKS-RS clients connected to Go hosts. Neither SECURITY.md:441-447 nor book/16-chat.qmd treats member-authored garbage as an availability risk. The recommendation only changes client-local verdicts, has no wire impact, and keeps digest-based substitution detection. Context the finding omits: Go v0.1.9's own client also fails a whole fetch on an undecryptable row (librt/minder.go decodeMsgs and decodeAndCacheServerMsgs), so this fix helps FOKS-RS clients only.

</details>

### chat-backend-attachments-via-team-kv

**Attachments stored in the team KV under channel-scoped roles, referenced from Basic text**

- Type: missing-feature
- Priority: medium
- Effort: L
- Layers: client-lib, agent, desktop-ui, protocol
- Verification: adjusted

Attachments exist only as an enum value and a capability bit. v0.1.9 has no Attachment body arm, and the format-2 RtChatPurpose has no attachment variant. Message bodies are capped near 1 MiB and fully replicated per message row. The team KV store already provides end-to-end encrypted large files with per-directory read and write roles and journaled uploads, so it can hold attachment content without a new server object store.

**Evidence**

- [`crates/foks-proto/src/realtime.rs:205`](../../../crates/foks-proto/src/realtime.rs#L205): Attachment=5 exists only as an enum value; RtMessageBody::validate (290-303) and from_value (337-353) accept only Basic and Pegged Edit/Reaction/Reply
- [`crates/foks-proto/src/chat_context.rs:71`](../../../crates/foks-proto/src/chat_context.rs#L71): RtChatPurpose has no attachment variant
- [`crates/foks-proto/src/realtime_extension.rs:34`](../../../crates/foks-proto/src/realtime_extension.rs#L34): attachments capability bit, valid only with content_actions (line 59)
- [`crates/foks-client/src/kv/write.rs:240`](../../../crates/foks-client/src/kv/write.rs#L240): put_file with KvWriteOptions (read/write roles); mkdir at line 455
- [`crates/foks-client-app/src/team.rs:1347`](../../../crates/foks-client-app/src/team.rs#L1347): Team KV root is created with read member(-0x4000) and write Role::OWNER, so members cannot create /.chat
- [`crates/foks-server/src/services/kv.rs:920`](../../../crates/foks-server/src/services/kv.rs#L920): Server requires the parent directory's write role for new entries

**Recommendation**

Phase 1 (Go-compatible): an owner provisions /.chat once, with read_role MIN_ROLE (member(-0x4000), the same as the root) and write_role at the lowest role allowed to create channels. Do this at team creation for new teams, and as an explicit owner action, or on an owner's first attachment send, for existing teams. Before the first upload, the channel creator creates /.chat/<channel-id> with read_role = channel roles.read and write_role = channel roles.write. Every client checks the directory's roles against the verified channel metadata before uploading or rendering, and refuses on mismatch (this covers squatting). Upload content to /.chat/<channel-id>/<operation-id>. Send a Basic message containing a readable line plus a 'foks-kv:' reference (path, size, media type, SHA-256 of the plaintext). FOKS-RS renders a card and verifies the hash after fetching; Go clients see the text and can fetch the file with Go KV tooling. The agent journals this as one two-step workflow (KV put, then chat send), with the KV path derived from the chat operation ID so recovery is idempotent. Attachment size is capped separately from CHAT_TEXT_BYTES. Document that any channel writer can overwrite or delete attachments in Phase 1, and show 'File no longer available' and 'Hash mismatch'. Phase 2: RtChatPurpose::Attachment {event, object_hash} in format 2 for server-enforced authorization and retention, tied to the Stage 1 retention floor.

**Already tracked:** ISSUES.md deferred extended-chat section ('attachment authorization and committed-object retention are designs'); the KV-backed Go-compatible path is new.

**Mockup:** [Extended chat preview: replies, files, channel settings, presence](../mockups/chat-capability-gated-extensions.html)

<details><summary>Verifier note</summary>

The core claim holds. RtMessageType::Attachment=5 has no body arm, and RtMessageBody::validate/from_value (realtime.rs:290-353) admit only Basic and Pegged Edit/Reaction/Reply. RtChatPurpose (chat_context.rs:71) has no attachment variant. The attachments bit requires content_actions (realtime_extension.rs:58-59). RT_MAX_BODY_BYTES is 1 MiB minus 1 KiB. put_file (write.rs:240) and mkdir (write.rs:455) take KvWriteOptions with read_role and write_role. The recommendation has a feasibility gap. The team KV root is created with write role Role::OWNER ('Share root discovery while retaining Owner-only root writes', crates/foks-client-app/src/team.rs:1345-1347). The server requires the parent directory's write role for every new entry (crates/foks-server/src/services/kv.rs:908-920, 288). So an ordinary member who sends an attachment cannot create /.chat or /.chat/<channel-id>. Two other details are missing. First, a member with write access to /.chat could create /.chat/<channel-id> with the wrong roles before the channel owner does. Second, write_role equal to roles.write lets any channel writer replace or remove another member's attachment. The SHA-256 check detects replacement but does not prevent deletion.

</details>

### chat-backend-capacity-refusals-become-uncertain

**Send-side capacity refusals are reported as generic 12001, leaving operations permanently Uncertain**

- Type: correctness-risk
- Priority: medium
- Effort: S
- Layers: server, client-lib, agent
- Verification: adjusted

Definite pre-commit refusals of realtime mutations are reported as the generic 12001 status. The reachable cases are the 256-channel-per-team cap and membership fanout above 1,024 (the latter only with a non-default maximum_team_members). The client treats 12001 as possibly committed, so the CreateChannel or Send operation becomes Uncertain. It cannot be cancelled, reconcile never finds it, and it holds one of the 1,000 pending slots for that team indefinitely. Each retry adds another slot. Upstream defines STATUS_OVER_QUOTA_ERROR (1060), and both RpcStatus::QuotaExceeded and DbError::QuotaExceeded already exist, but realtime uses neither. Several chat paths hard-code status literals.

**Evidence**

- [`crates/foks-server/src/services/realtime.rs:290`](../../../crates/foks-server/src/services/realtime.rs#L290): DbError::Capacity(_) => RpcStatus::Realtime (12001)
- [`crates/foks-server-db/src/realtime/inbox.rs:28`](../../../crates/foks-server-db/src/realtime/inbox.rs#L28): Fanout capacity error raised before commit
- [`crates/foks-server-db/src/realtime/channels.rs:111`](../../../crates/foks-server-db/src/realtime/channels.rs#L111): Channel-count capacity raised before commit
- [`crates/foks-client/src/realtime/operations/attempt.rs:128`](../../../crates/foks-client/src/realtime/operations/attempt.rs#L128): definite_rejection lists 1013|1030|12002|12003|12005|12006 as literals; 12001 and 1060 retain uncertainty
- [`crates/foks-client-db/src/repositories/chat.rs:364`](../../../crates/foks-client-db/src/repositories/chat.rs#L364): Only Prepared operations can be cancelled
- [`crates/foks-rpc/src/generated/status_codes.rs:29`](../../../crates/foks-rpc/src/generated/status_codes.rs#L29): STATUS_OVER_QUOTA_ERROR = 1060 exists upstream
- [`crates/foks-client/src/realtime/operations/recovery.rs:98`](../../../crates/foks-client/src/realtime/operations/recovery.rs#L98): Literal 12001 for window shrinking
- [`crates/foks-agent/src/chat.rs:677`](../../../crates/foks-agent/src/chat.rs#L677): Literal 12001 and 1013 (line 859) instead of STATUS_* constants

**Recommendation**

At the admission sites (channel count in channels.rs:111 and 159, fanout in inbox.rs:28 when reached from a mutation, and the defensive stored-size checks in messages.rs:100-116) return DbError::QuotaExceeded instead of Capacity. Add a realtime db_error arm mapping QuotaExceeded to RpcStatus::QuotaExceeded (1060). Keep Capacity and 12001 for read-response bounds, because history window halving depends on them. Add STATUS_OVER_QUOTA_ERROR (and STATUS_RATE_LIMIT_ERROR, if the FOKS-RS rate limiter runs before dispatch) to definite_rejection. Replace the literals in attempt.rs, recovery.rs and agent chat.rs with generated STATUS_* constants. Add conformance tests in which creating channel 257 ends Rejected(1060) and a send to a team above the fanout bound ends Rejected(1060), not Uncertain. Optionally, warn in the client or desktop before attempting a create at the channel cap. A user-initiated 'retry exact' for Uncertain sends can follow separately, behind a capability, since only the FOKS-RS server's idempotent replay (messages.rs:51-63) supports it.

**Already tracked:** ISSUES.md Stage 1 (quotas) and Stage 6 (fanout > 1,024) describe the limits but not the status mapping or the resulting uncertain operations.

**Mockup:** [Delivery, sender identity and integrity states](../mockups/chat-delivery-and-integrity.html)

<details><summary>Verifier note</summary>

The core claim holds. db_error maps DbError::Capacity to RpcStatus::Realtime, which returns code 12001 (services/realtime.rs:290, response.rs:74). The fanout check (inbox.rs:28, through stamp) and the channel-count check (channels.rs:111) both run inside the uncommitted Immediate transaction. definite_rejection lists only 1013|1030|12002|12003|12005|12006 (attempt.rs:130-138). chat_cancel allows only state 0 or 4 (repositories/chat.rs:364-375). reconcile_operation keeps uncertainty when the message is absent (recovery.rs:6). Pending admission counts rows with cleanup_pending=1 (repositories/chat.rs:150-162). The literals are confirmed at recovery.rs:98, agent chat.rs:677 and agent chat.rs:859. Priority should be lower. The reachable refusals are the 256-channel cap, which no client or desktop check catches first, and fanout above 1,024 members, which needs a non-default maximum_team_members (default 64). The stored-size checks are effectively unreachable, because proto validation bounds ciphertext below STORED_MESSAGE_BYTES (limits.rs asserts RT_MAX_CIPHERTEXT_BYTES+512 <= STORED_MESSAGE_BYTES). Two details simplify the fix. foks-server-db already has DbError::QuotaExceeded, used by KV, team names and invitations, but realtime db_error currently sends it to the `_ => TransactionRetry` arm (1014), which would also leave the operation uncertain. Go v0.1.9's realtime server never emits 1060 (server/realtime/*.go), so treating 1060 as a definite rejection is safe against Go hosts.

</details>

### chat-backend-channel-management-format1

**Channel rename, description, posting role and archive via a Rust-only update method that keeps Go wire metadata**

- Type: missing-feature
- Priority: medium
- Effort: L
- Layers: protocol, server, client-lib, agent, desktop-ui
- Verification: adjusted

Channel configuration is immutable by schema and code. The Go v0.1.9 protocol has no update method, and the capability rules require format 2 for channel_management. However, the Go RtChannelMetadata already carries sequence (always 1 today) and updated_at, so a server-side revision is readable by Go clients through rtListAllChannelsForTeam with no schema change. Three constraints apply. Format-1 name boxes are not bound to a revision, so a server could replay an old name. Reconcile of an uncertain create compares the full metadata and would report a later rename as an integrity error. history.rs requires every message box's role to equal the channel's current read role, so a read-role change would make all earlier history unreadable.

**Evidence**

- [`crates/foks-server-db/src/schema/realtime.sql:16`](../../../crates/foks-server-db/src/schema/realtime.sql#L16): 'Immutable configuration above'; format immutable trigger at line 22
- [`crates/foks-server-db/src/schema/realtime_invalidation.sql:3`](../../../crates/foks-server-db/src/schema/realtime_invalidation.sql#L3): 'Future configuration edits/deletes must invalidate or fan out transactionally'
- [`crates/foks-server-db/src/realtime/channels.rs:51`](../../../crates/foks-server-db/src/realtime/channels.rs#L51): Creation requires md.sequence == 1
- [`crates/foks-proto/src/realtime.rs:495`](../../../crates/foks-proto/src/realtime.rs#L495): RtChannelMetadata has sequence and updated_at fields
- [`crates/foks-client/src/realtime/operations/recovery.rs:24`](../../../crates/foks-client/src/realtime/operations/recovery.rs#L24): Uncertain create compares the entire metadata except times
- [`crates/foks-client/src/realtime/history.rs:307`](../../../crates/foks-client/src/realtime/history.rs#L307): Message box role must equal md.roles.read
- [`crates/foks-proto/src/realtime_extension.rs:54`](../../../crates/foks-proto/src/realtime_extension.rs#L54): channel_management requires extended_channels

**Recommendation**

Add foksChatUpdateChannel once a durable vendor method namespace is assigned, as ISSUES.md already requires before any extension capability is advertised. Gate it with a channel_update bit carried in a version-2 capabilities response, because the current response is a strict 8-field version 1 that fails closed. Arguments: {channel, expected_sequence, set_version, name?: RtBox, description?: RtBox, write_role?: Role, archived?: bool}. In one immediate transaction the server authorizes (admin+ for the admin tier; a policy is still needed for the bottom tier), checks expected_sequence and the set_version CAS as rt_create_channel does, keeps id, team, tier, format and roles.read immutable, writes metadata with sequence+1 and updated_at set to the new set version, and stamps readers so soft-store rows pick up the new metadata. Before relying on Go readability, add oracle cases showing that a v0.1.9 client accepts list and delta metadata with sequence > 1 and a changed name box. Archive: write_role = OWNER plus a Rust-only flag. Go clients see a channel restricted to owners, not one that is fully read-only, so archived channels must be enforced as read-only in the Rust client and server. Defer delete until the retention floor exists. Client: a hard-state chat_channel_revisions table (scope, max_sequence) that rejects metadata whose sequence decreases. This limits name replay to the latest revision but cannot prove freshness, because format-1 name boxes are not bound to a revision. Uncertain-create reconcile compares only immutable fields. Add a new ChatOperationKind and extend the CHECK at schema.rs:83. Agent IPC: PrepareChannelUpdate, reusing Status, Attempt and Reconcile. Read-role changes stay format-2 only.

**Already tracked:** ISSUES.md 'Existing disclosed limitations' (extended channel management disabled); the format-1 revision path and its client constraints are new.

**Mockup:** [Extended chat preview: replies, files, channel settings, presence](../mockups/chat-capability-gated-extensions.html)

<details><summary>Verifier note</summary>

The core claims hold. realtime.sql:16 ('Immutable configuration above') and the format-immutable trigger. realtime_invalidation.sql:3-4. channels.rs requires md.sequence == 1. RtChannelMetadata has sequence and updated_at. Uncertain-create reconcile compares the full metadata apart from ctime, mtime and last_message (recovery.rs:23-40). history.rs:307 requires the box role to equal md.roles.read. channel_management requires extended_channels (realtime_extension.rs:53-55). open_text binds the name box only to the key and purpose, so an old name can be replayed. chat_operations.kind is CHECK(kind IN (0,1)) at schema.rs:83. The desktop already has ChannelInfoPanel and an 'Edit channel' control marked 'Editing channels is not implemented', so this is a reasonable mockup target. The recommendation needs three corrections. (a) Archive as write_role = OWNER does not make the channel read-only for Go clients, because owners can still post. (b) The capability response is fixed at version 1 with 8 fields and fails closed on unknown shapes. ISSUES.md also requires a durable method namespace before any extension capability is advertised, so 'the vendor method namespace' does not exist yet. (c) That Go v0.1.9 clients tolerate a channel whose metadata later changes (sequence > 1, new updated_at) is unverified, because no Go sources are available locally, and it should be pinned with the oracle before the plan relies on it.

</details>

### chat-backend-notification-prefs-mentions-names

**Agent-owned per-channel alert/hide preferences synced via personal KV, local mentions and sender labels**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: agent, client-lib, desktop-ui, desktop-native
- Verification: confirmed

The FOKS-RS server always returns hidden=false and muted=false. Neither the Go nor the Rust protocol has a setter. The client soft store overwrites hidden and muted from each delta, so they cannot hold local state. The desktop native shell keeps a separate device-local on/off override per channel, but unread badges use the server's muted and hidden flags, so a muted channel still counts toward unread. That leaves two sources of preference, neither user-settable for badges, and none shared across devices. There is no mentions support. The agent returns sender UIDs in hex only, and the desktop resolves names from the current roster, so former members appear as short IDs.

**Evidence**

- [`crates/foks-server-db/src/realtime/inbox.rs:535`](../../../crates/foks-server-db/src/realtime/inbox.rs#L535): hidden: false, muted: false hard-coded in deltas
- [`crates/foks-client-db/src/soft.rs:346`](../../../crates/foks-client-db/src/soft.rs#L346): Delta upsert overwrites hidden/muted
- [`apps/desktop/src-tauri/src/commands/chat_local.rs:20`](../../../apps/desktop/src-tauri/src/commands/chat_local.rs#L20): Device-local Settings.overrides: BTreeMap<String, bool>
- [`apps/desktop/src/chat/unread.ts:12`](../../../apps/desktop/src/chat/unread.ts#L12): Unread badge excludes only server-provided hidden/muted
- [`crates/foks-agent/src/chat.rs:716`](../../../crates/foks-agent/src/chat.rs#L716): History sender is a hex UID
- [`apps/desktop/src/chat/presentation.ts:114`](../../../apps/desktop/src/chat/presentation.ts#L114): Names come from the current team roster only
- [`crates/foks-proto/src/realtime_extension.rs:31`](../../../crates/foks-proto/src/realtime_extension.rs#L31): Unused 'preferences' capability bit

**Recommendation**

(1) Add a hard-state table chat_channel_preferences(host, uid, team, channel, alerts IN ('all','mentions','none'), hidden, updated_at) that delta application never touches. The agent merges it into ChatConversation (muted = alerts == 'none'), so badge and notification policy have one source. Migrate the desktop native overrides into it once. (2) Sync across the user's devices through the user's personal KV namespace at a reserved path, last writer wins per channel by updated_at. This is Go-compatible and server-blind and needs no realtime protocol change. Leave the 'preferences' capability unused unless server-side state proves necessary. (3) Mentions: plain '@username' tokens in Basic text, which Go clients display as text. The agent resolves them against the verified roster and sets a mentions_actor flag on history and preview rows, which makes 'mentions only' work locally. (4) Add sender_label to history rows, resolved from the verified roster with a 'former member' fallback that keeps the last verified username.

**Already tracked:** ISSUES.md 'Existing disclosed limitations' lists mentions as disabled; preferences, mute semantics and sender labels are not tracked.

**Mockup:** [Channel audience and alert settings](../mockups/chat-channel-info-and-alerts.html)

<details><summary>Verifier note</summary>

All claims verified. The server hard-codes hidden: false and muted: false (inbox.rs:535-536). The soft store overwrites hidden and muted on each delta upsert (soft.rs:347-348). Native Settings.overrides is a BTreeMap<String, bool> keyed by a hashed scope and channel, and the UI exposes inherit, all and none as 'Alerts' (notification-provider.tsx). teamUnread and the chat-teams fold sum exclude only the server-provided hidden and muted flags, so a channel with alerts set to 'None' still counts as unread. Upstream v0.1.9 has no hidden or muted setter. The agent returns the sender as hex (chat.rs:716). partyNames resolves names only from snapshot.parties for the store and falls back to shortId. No mention or sender-label support exists in the agent, client or desktop. The preferences capability bit is unused. ISSUES.md lists only mentions as disabled. The recommendation is feasible: KV supports user-party namespaces and needs no realtime protocol change. One open design point: the format-2 design in chat_context.rs:240 keeps mention identities encrypted, so plain-text '@username' tokens should be documented as a format-1 convention that format-2 structured mentions will replace.

</details>

### chat-backend-retention-floor-vs-anchor-ledger

**Define retention as an explicit history floor; the client's gap, predecessor and recovery logic assume dense sequences**

- Type: correctness-risk
- Priority: medium
- Effort: M
- Layers: server, client-lib, protocol
- Verification: adjusted

Any Stage 1 retention that deletes rt_messages rows will conflict with client logic that assumes every sequence from 1 to head exists. read_after reports a gap whenever the returned range is shorter than head - start + 1, so an incremental read whose cursor is below the deletion point is flagged and the desktop falls back to a full recents reload. This happens once per occurrence, not indefinitely. A client with no retained anchor for a predecessor below the deletion point reports it as missing_predecessors, which is the same signal as a server withholding messages. Uncertain-send recovery scans forward from scan_cursor in 100-sequence windows and does not advance on an empty page, so an operation more than one window below the deletion point stays Uncertain indefinitely without any error. rt_read_through returns NotFound for a sequence whose row has been deleted. A tombstone that changes a message's encoding changes its anchor digest, which chat_accept_page rejects as a contradiction. Retention therefore has to be visible to the client as policy, not inferred from missing rows.

**Evidence**

- [`crates/foks-client/src/realtime/history.rs:92`](../../../crates/foks-client/src/realtime/history.rs#L92): gap |= rows.len() != head - start + 1
- [`apps/desktop/src/chat/use-chat-history.ts:121`](../../../apps/desktop/src/chat/use-chat-history.ts#L121): Any gap triggers one full recents reload that replaces the window
- [`crates/foks-client/src/realtime/history.rs:199`](../../../crates/foks-client/src/realtime/history.rs#L199): Predecessors that cannot be fetched and have no anchor are reported as missing_predecessors
- [`crates/foks-client/src/realtime/operations/recovery.rs:80`](../../../crates/foks-client/src/realtime/operations/recovery.rs#L80): An empty or short window does not advance scan_cursor, so a scan below the floor stalls
- [`crates/foks-server-db/src/realtime/inbox.rs:348`](../../../crates/foks-server-db/src/realtime/inbox.rs#L348): rt_read_through returns NotFound when the row is absent
- [`crates/foks-client-db/src/repositories/chat.rs:450`](../../../crates/foks-client-db/src/repositories/chat.rs#L450): Any digest change for a retained (sequence, id) is rejected as ChatConflict

**Recommendation**

Specify retention per channel as a monotonic low-water mark, history_floor, never as arbitrary row deletion. The server deletes rows below the floor in bounded writer batches and keeps last_sequence. Expose the floor through a capability-gated Rust-only call, for example foksChatChannelState {channel} returning {floor, head}, or as a field of the update or GetChannel extension. Go clients see only fewer rows. Client changes: treat sequences below the floor as expired, not missing, in the read_after gap rule and the predecessor checks; prune anchors below the floor; start recovery at max(scan_cursor, floor) and surface an Uncertain send whose window fell below the floor as 'outcome unknowable after retention' without dropping it. Server: accept read_through for any sequence up to head, even below the floor. Per-message user deletion stays in format 2 (the Delete purpose exists in chat_context.rs). It needs a ledger rule that lets an anchored digest move once to a tombstone digest, only with an authorized delete event.

**Already tracked:** ISSUES.md Stage 1 ('choose explicit retention, archival and deletion behavior'); adds the floor protocol and the client-side consequences.

<details><summary>Verifier note</summary>

The core claim holds. read_after sets gap when rows.len() != head - start + 1 (history.rs:92). verify_page reports predecessors that cannot be fetched as missing_predecessors (history.rs:199-277). rt_read_through returns NotFound when the row is absent (inbox.rs:348-361). chat_accept_page rejects any change in a retained (sequence, id) digest (chat.rs:450). ISSUES.md Stage 1 tracks only the general need to 'choose explicit retention, archival and deletion behavior', so the floor protocol and the client consequences are new, and the recommendation fits the project constraints. Two statements in the summary are wrong. First, reloads do not repeat indefinitely: on gap !== false the desktop reloads recents once and replaces the window (use-chat-history.ts:121-131), so later incremental reads start from the new head. Second, recovery does not fail with NotFound. rt_thread returns an empty range for deleted rows, and reconcile_operation leaves scan_cursor where it is on an empty page (recovery.rs:80-81). An Uncertain send whose cursor is more than one window below the deletion point therefore stalls permanently and is never reported as an error.

</details>

### chat-backend-revocation-degrades-inbox

**The FOKS-RS server itself puts an inbox into the degraded state on every access revocation**

- Type: correctness-risk
- Priority: medium
- Effort: S
- Layers: server, client-lib, protocol
- Verification: confirmed

When reconciliation removes a channel from a user's inbox, remove_user_channel increments the user's inbox version. rt_changed_threads filters out rows with accessible=0, format-2 rows and rows the user can no longer read. If the most recent change is a revocation, the delta from the client's cursor is empty while the server's version is ahead. After its one retry, the client marks the scope degraded, and the agent never sets the drain gate. ISSUES and SECURITY.md attribute this behavior to the pinned Go server, but the server-db test shows the Rust server produces it too: version 7 with only one older row.

**Evidence**

- [`crates/foks-server-db/src/realtime/inbox.rs:129`](../../../crates/foks-server-db/src/realtime/inbox.rs#L129): remove_user_channel bumps the inbox version and sets accessible=0
- [`crates/foks-server-db/src/realtime/inbox.rs:480`](../../../crates/foks-server-db/src/realtime/inbox.rs#L480): Delta query filters accessible=1 AND format=1
- [`crates/foks-server-db/src/realtime/tests.rs:921`](../../../crates/foks-server-db/src/realtime/tests.rs#L921): After demotion, inbox_version 7 is returned with one row at a lower version
- [`crates/foks-client/src/realtime/inbox.rs:544`](../../../crates/foks-client/src/realtime/inbox.rs#L544): Empty delta above the cursor: retry once at 1,000 rows, then observe head as degraded
- [`crates/foks-agent/src/chat.rs:389`](../../../crates/foks-agent/src/chat.rs#L389): Degraded drains are not recorded, so every team sync re-queries the version

**Recommendation**

Option A (preferred, no change to Go wire semantics): add an exact_filtered_inbox capability bit. With it, the FOKS-RS server guarantees that rows between the cursor and head are filtered only because they are inaccessible or format 2. A client talking to such a host advances its cursor to head on an empty delta without marking the scope degraded, and still uses listing-based retain_chat_inbox_team to drop the channel. Option B: return revoked rows projected through ChannelPolicy::project as unreadable, so the cursor advances naturally; this requires checking Go client handling of unreadable delta rows with the oracle. Add a conformance test: demote a member, sync, and assert the result is not degraded and the channel is removed.

**Already tracked:** ISSUES.md 'Existing disclosed limitations' and crates/foks-client/SECURITY.md:444-447 cover the Go filtered-inbox case only; the Rust-server trigger and the fix are new.

<details><summary>Verifier note</summary>

All claims verified. remove_user_channel calls next_inbox_version and sets accessible=0 (inbox.rs:129-156). rt_changed_threads filters on accessible=1 AND format=1 and skips rows the user can no longer read (inbox.rs:480, 522-528). The server-db test asserts inbox_version 7 with a single, older row after demotion (tests.rs:921-923). On an empty delta above the cursor, drain_inbox_scope retries once at 1,000 rows and then calls observe_chat_inbox_head with degraded=true (client inbox.rs:544-553). record_drain skips degraded drains (agent chat.rs:389). ISSUES.md:174 and SECURITY.md:444-447 attribute this behavior only to the pinned Go inbox. The degraded state clears at the next accessible change, but until then the unread badges show '+' or '?' (unread.ts). Option A fits the project constraints. The capability response is strictly versioned (version 1, 8 fields, fails closed), so the new bit needs a response version bump.

</details>

### chat-backend-stage1-redundant-envelope-and-counters

**Stage 1 quota: drop the redundant envelope column and keep retained-byte counters in the send transaction**

- Type: performance
- Priority: medium
- Effort: M
- Layers: server
- Verification: adjusted

Each rt_messages row stores the RtSend envelope (expected_previous_sequence zeroed) and the exact RtMessage. The envelope is used only for the idempotent-replay comparison and can be rebuilt exactly from exact_message plus the channel's short ID, so the ciphertext is stored twice. Admission against a per-channel or per-team budget would need a SUM over rt_messages unless the server keeps counters. ISSUES requires that idempotency recovery and admission share one writer transaction, but it gives no mechanism.

**Evidence**

- [`crates/foks-server-db/src/schema/realtime.sql:32`](../../../crates/foks-server-db/src/schema/realtime.sql#L32): rt_messages.envelope and exact_message (line 33), each CHECK length <= 1048576
- [`crates/foks-server-db/src/realtime/messages.rs:47`](../../../crates/foks-server-db/src/realtime/messages.rs#L47): envelope = RtSend with expected_previous_sequence = 0, used only in the replay comparison (lines 47-63)
- [`crates/foks-server-db/src/realtime/messages.rs:105`](../../../crates/foks-server-db/src/realtime/messages.rs#L105): Both encodings inserted
- [`crates/foks-server-db/src/realtime/limits.rs:3`](../../../crates/foks-server-db/src/realtime/limits.rs#L3): Only per-request and per-response bounds; no retained budget
- [`crates/foks-server-db/src/schema.rs:44`](../../../crates/foks-server-db/src/schema.rs#L44): In-place schema upgrades (43..47 to SCHEMA_VERSION 48); a storage change needs a new version step with backfill

**Recommendation**

Ship this as a schema version 49 step in schema.rs's upgrade transaction, with no wire impact. (1) Drop the envelope column, or replace it with a 32-byte envelope_digest computed from the existing envelope during the upgrade. On replay, either decode exact_message and rebuild RtSend{metadata, channel: short_id, wrapper, expected_previous_sequence: 0} for comparison, or compare digests. (2) Add retained_rows and retained_bytes to rt_channels, and team totals to rt_channel_sets. Backfill them in the same upgrade with one SUM over rt_messages. Update them in rt_send's immediate transaction after the replay branch, so recovering an existing record never hits quota. (3) Check per-channel, per-team and operator-wide chat budgets from server config before the INSERT, and refuse with DbError::QuotaExceeded mapped to 1060 (see the capacity finding). (4) Export the counters through crates/foks-server/src/metrics/realtime.rs. (5) Test that a team at its budget cannot block account or KV writes, that an exact replay of an already-stored message succeeds at quota, and that upgrading a populated v48 database yields correct counters.

**Already tracked:** ISSUES.md Stage 1 (adds the envelope deduplication and counter mechanism, which are not stated there).

<details><summary>Verifier note</summary>

The core claim holds. rt_messages stores both envelope and exact_message, each up to 1 MiB. The envelope column is read only by the replay comparison in rt_send (messages.rs:47-63; no other code reads it). The envelope equals RtSend{metadata, channel short_id, wrapper, expected_previous_sequence: 0}, and every field is recoverable from exact_message plus rt_channels.short_id, because the replay branch already checks that the channel matches. The counter approach (rt_channels and rt_channel_sets updated after the replay branch, refusal with 1060) fits ISSUES.md Stage 1's requirement that admission and idempotency recovery share one transaction, and adds a concrete mechanism that ISSUES lacks. Two corrections. The server keeps an in-place upgrade path (schema.rs:6 SCHEMA_VERSION=48, with upgrades from 43-47 at schema.rs:44-111), so this is a schema-version bump that must rewrite or drop the envelope column and backfill counters from existing rows; otherwise counters start wrong on existing hosts. Calling it simply a 'pre-v1 schema change' understates that. The realtime.sql evidence line also needs correcting: rt_messages begins at line 27, and the envelope and exact_message columns are lines 32-33, not line 26.

</details>

### chat-backend-stage2-submission-floor

**Stage 2 compaction: time-ordered submission IDs plus a durable floor make expiry safe**

- Type: maintainability
- Priority: medium
- Effort: M
- Layers: client-lib, agent, desktop-ui
- Verification: adjusted

chat_operations and chat_submissions have no timestamps. Submission IDs are 16 random bytes supplied by the desktop, so if terminal rows are deleted, an old submission ID replayed later would look new and be sent again. ISSUES requires that 'deletion must not make an old submission identifier appear new' but names no mechanism. Operation IDs are minted by the agent and never accepted from callers for creation, so only submissions need protection.

**Evidence**

- [`crates/foks-client-db/src/schema.rs:77`](../../../crates/foks-client-db/src/schema.rs#L77): chat_operations and chat_submissions (lines 77-104) have no lifecycle timestamps
- [`crates/foks-client-db/src/repositories/chat.rs:81`](../../../crates/foks-client-db/src/repositories/chat.rs#L81): chat_submission lookup by (scope, submission_id) returns None for an unknown ID
- [`crates/foks-agent-proto/src/chat.rs:281`](../../../crates/foks-agent-proto/src/chat.rs#L281): valid_chat_id accepts any nonzero 32-hex ID for submissions
- [`apps/desktop/src/chat/actions.ts:15`](../../../apps/desktop/src/chat/actions.ts#L15): submissionId() mints IDs with crypto.randomUUID(), a random UUIDv4 with no time ordering
- [`crates/foks-agent-proto/src/chat.rs:15`](../../../crates/foks-agent-proto/src/chat.rs#L15): SaveIntent/ImportIntent keep caller submission IDs in agent storage with no time limit
- [`crates/foks-client-app/src/chat.rs:29`](../../../crates/foks-client-app/src/chat.rs#L29): chat_submission MAC binds input to the caller-chosen ID

**Recommendation**

Make submission IDs time-ordered (48-bit millisecond timestamp plus 80 random bits, a UUIDv7 layout). Either mint them in the agent (ChatAction::NewSubmission), or keep desktop minting in apps/desktop/src/chat/actions.ts and validate the timestamp against a bounded clock-skew window (desktop and agent share a clock). Add terminal_at to chat_operations. A compaction job deletes terminal, cleaned-up operations and their submission rows older than the retention window, in bounded batches. In the same transaction it raises a durable per-(host, uid, team) submission_floor. The floor never passes the oldest submission ID referenced by a saved intent in PendingChatStore, or by any Prepared or Uncertain operation. A lookup with no row for an ID below the floor returns a distinct SubmissionExpired error and never prepares a new operation. The desktop then asks the user before resending under a new ID. Optionally keep a narrow evidence table (submission_id, input_mac, outcome, receipt) for a longer replay window.

**Already tracked:** ISSUES.md Stage 2; adds the concrete expiry mechanism.

<details><summary>Verifier note</summary>

The core claim holds. chat_operations and chat_submissions (schema.rs:77-104) have no lifecycle timestamps. chat_submission (repositories/chat.rs:81) returns None when no row exists, which a caller treats as a new submission. ISSUES.md Stage 2 (lines 38-64) requires 'deletion must not make an old submission identifier appear new' but names no mechanism, so the time-ordered ID plus durable floor is a concrete addition. Corrections: (1) valid_chat_id is at crates/foks-agent-proto/src/chat.rs:281, not 291. (2) The desktop mints submission IDs with crypto.randomUUID() (apps/desktop/src/chat/actions.ts:15), a UUIDv4 with about 122 random bits, so desktop-ui is an affected layer, and the layer list repeated client-lib. (3) The recommendation misses saved intents. SaveIntent and ImportIntent keep a submission ID and its text in the agent's PendingChatStore with no time limit, and the desktop resubmits saved drafts after restart. A floor raised past such an ID would turn an unsent draft into SubmissionExpired. An intent left behind when the desktop crashed before ClearIntent is indistinguishable from a compacted send, so the floor must also stay below the oldest submission ID still held by a saved intent.

</details>

### chat-backend-capability-negotiation-not-forward-compatible

**Chat capability discovery fails closed on version skew, forces format 2 for every feature, and is unused by the agent**

- Type: correctness-risk
- Priority: low
- Effort: S
- Layers: protocol, server, client-lib
- Verification: adjusted

The chat capability exchange is strict by design. Request and response versions must equal 1, the response is a fixed 8-field array, and the v1 dependency rules tie every feature to extended_channels (format 2). The client treats only MethodNotFound and NOT_IMPLEMENTED as basic_only. Outside the live_compat example nothing calls ChatSession::capabilities, and the agent has no IPC action for it. Because the request carries a version, a later server can keep answering v1 requests unchanged, so older clients are not broken. The gap is in the other direction and in documentation. A newer client that sends v2 to a v1 server receives BadArguments, and no fallback or negotiation rule is specified. Nothing is advertised yet, so this is design hardening to settle with the deferred method-namespace assignment, not a current correctness risk.

**Evidence**

- [`crates/foks-proto/src/realtime_extension.rs:16`](../../../crates/foks-proto/src/realtime_extension.rs#L16): Request version must equal 1
- [`crates/foks-proto/src/realtime_extension.rs:53`](../../../crates/foks-proto/src/realtime_extension.rs#L53): Every feature bit requires extended_channels; threads/attachments require content_actions
- [`crates/foks-proto/src/realtime_extension.rs:78`](../../../crates/foks-proto/src/realtime_extension.rs#L78): Response must be exactly 8 fields with version 1
- [`crates/foks-client/src/realtime/capabilities.rs:29`](../../../crates/foks-client/src/realtime/capabilities.rs#L29): Only MethodNotFound/NOT_IMPLEMENTED fall back to basic_only; decode errors propagate
- [`crates/foks-server/src/services/realtime.rs:226`](../../../crates/foks-server/src/services/realtime.rs#L226): Server always returns basic_only regardless of request
- [`crates/foks-client/examples/live_compat.rs:83`](../../../crates/foks-client/examples/live_compat.rs#L83): Sole caller of ChatSession::capabilities
- [`crates/foks-agent-proto/src/chat.rs:9`](../../../crates/foks-agent-proto/src/chat.rs#L9): ChatAction has no capabilities query

**Recommendation**

Settle negotiation rules in the same change that ISSUES.md 'Deferred extended-chat prerequisite' requires for the durable method namespace. Either (a) the server accepts any request version >= 1 and answers with min(requested, server_max), or (b) a client sending v>1 retries once with v1 on BadArguments. Consider replacing the boolean array with a u64 feature bitmask in which clients ignore unknown bits and validate dependencies only for bits they know. Define dependency rules per version. Do not define a format-1 pegged_content bit, because Go v0.1.9 clients cannot read non-Basic bodies. Add ChatAction::Capabilities, cached per CheckedProfileSession and dropped on identity change, only when the first bit is advertised. Add tests for v1 client against v2 server and v2 client against v1 server.

**Already tracked:** ISSUES.md 'Deferred extended-chat prerequisite' covers only the method-position namespace; version negotiation, unknown-bit tolerance, dependency rules and agent exposure are not tracked.

**Mockup:** [Extended chat preview: replies, files, channel settings, presence](../mockups/chat-capability-gated-extensions.html)

<details><summary>Verifier note</summary>

These facts hold: request version must be 1 (realtime_extension.rs:16); the response is exactly 8 fields at version 1 (lines 78-79); in v1 every feature bit depends on extended_channels (lines 53-61); the client falls back only on MethodNotFound or NOT_IMPLEMENTED (capabilities.rs:31-38); the server always returns basic_only (services/realtime.rs:226-233); the only ChatSession::capabilities caller is live_compat.rs:83; ChatAction has no capabilities action. Three claims are overstated. First, 'a server that adds one bit breaks every older client' is wrong: the request carries its version, so a later server can keep answering v1 requests in the v1 shape. Only a v2 client against a v1 server fails, and that is a client fallback rule the v2 client can carry. Second, the doc comment states that v1 describes format 2 and fails closed by design, so format-1 features would go under a new version rather than being ruled out. Third, nothing is advertised and no product path reads the result, so this is not a current high-priority correctness risk. The recommendation also lists pegged_content as a format-1 bit. That is unsafe: Go v0.1.9 clients fail the whole thread fetch on any non-Basic body (librt/minder.go openMessage returns VersionNotSupportedError, and decodeMsgs propagates it).

</details>

### chat-backend-get-channel-read-path

**Every chat read lists and decrypts all channels; implement upstream rtGetChannel and reuse reads for mark-read**

- Type: performance
- Priority: low
- Effort: M
- Layers: protocol, server, client-lib, agent
- Verification: adjusted

ChatSession::channel() calls ListChannels and opens every channel's name and description just to look up one channel. History reads, read_after, mark_read, send preparation, attempt and reconcile all go through it, so a team with 200 channels costs a full server-side projection of all rows and up to about 400 client-side secretbox opens per history page. Upstream defines rtGetChannel (position 1, returning RTChannelMetadata), but neither foks-rpc nor the server implements it. mark_chat_read uses with_chat rather than with_chat_read, so each read-pointer update re-authenticates and reloads the team without the session read caches.

**Evidence**

- [`crates/foks-client/src/realtime/session.rs:311`](../../../crates/foks-client/src/realtime/session.rs#L311): channel() lists all channels and searches by ID
- [`crates/foks-client/src/realtime/session.rs:238`](../../../crates/foks-client/src/realtime/session.rs#L238): open_channel decrypts name and description per channel
- [`crates/foks-server/src/rpc/generated/routes.rs:1981`](../../../crates/foks-server/src/rpc/generated/routes.rs#L1981): rtGetChannel position 1 is an upstream method marked supported: false
- [`crates/foks-rpc/src/realtime.rs:14`](../../../crates/foks-rpc/src/realtime.rs#L14): RealtimeRequest has no GetChannel variant
- [`crates/foks-client-app/src/chat.rs:228`](../../../crates/foks-client-app/src/chat.rs#L228): mark_chat_read uses with_chat (fresh authenticate + team load)
- [`tools/foks-v019-oracle/realtime_fixture_test.go:172`](../../../tools/foks-v019-oracle/realtime_fixture_test.go#L172): Fixture cases skip position 1

**Recommendation**

(0) Measure first. The existing benchmark shows p95 rising only from 43.7 ms to 45.1 ms between 70 and 200 channels, so profile how much of a history read ListChannels and open_channel take before changing the protocol. (1) If the cost is significant, add position 1 to the oracle fixture cases to pin the exact argument shape, then add RtGetChannelArgument and the response to foks-proto and foks-rpc. (2) Server handler: channel(), require_basic, then ChannelPolicy::project on a reader snapshot. (3) ChatSession::channel() calls GetChannel and falls back to ListChannels on MethodNotFound, keeping the validate_channel checks. Note that the set-level duplicate and updated_at <= set.version checks are lost on this path. (4) A cheaper first step that needs no protocol change: an agent-process cache of decrypted channel names keyed by (channel id, exact name RtBox bytes, role, generation), following PreviewCache. (5) Moving mark_chat_read to with_chat_read requires revising the documented rule in set_read_reuse that operations which post anything authenticate inline. The server re-authorizes read-through, so the risk is limited to the client-side role check, but state the change explicitly as a policy change.

<details><summary>Verifier note</summary>

The factual claims hold. ChatSession::channel() calls list_current_channels and searches the result (session.rs:311-323). open_channel opens the name and, when the channel is readable, the description for every channel (238-268). History, read_after, mark_read, prepare, attempt, protected_request and recovery all call channel(). RealtimeRequest has no GetChannel variant. routes.rs marks rtGetChannel at position 1 as supported: false, and the upstream JSON gives its result as RTChannelMetadata. The oracle fixture skips position 1. mark_chat_read uses with_chat (client-app chat.rs:228). Two corrections apply. First, the priority is too high: README.md reports p95 of 43.7 ms at 70 channels and 45.1 ms at 200 channels for the real-process benchmark, which includes foreground history reads, so the measured cost of channel count is small. Second, mark_chat_read using with_chat is a documented choice, not an oversight. set_read_reuse says reuse 'is set only for read operations; a chat session that posts anything keeps authenticating inline'.

</details>

### chat-backend-history-search

**Bounded verified history search in the agent, without server or plaintext persistence**

- Type: missing-feature
- Priority: low
- Effort: M
- Layers: agent, client-lib, desktop-ui
- Verification: confirmed

There is no search. The server stores ciphertext only, the agent persists no message plaintext, and the desktop sees 50-row pages. A first version can scan backward through verified history pages in the agent and match in memory. This reuses the anchor-checked read path and needs no protocol change. An encrypted local index can come later if scan cost proves too high.

**Evidence**

- [`crates/foks-agent-proto/src/chat.rs:9`](../../../crates/foks-agent-proto/src/chat.rs#L9): ChatAction has History/NotificationHistory but no search
- [`crates/foks-agent-proto/chat-limits.json:4`](../../../crates/foks-agent-proto/chat-limits.json#L4): CHAT_PAGE_ROWS 50
- [`crates/foks-agent/src/chat.rs:250`](../../../crates/foks-agent/src/chat.rs#L250): Agent keeps decrypted previews only in bounded process memory
- [`crates/foks-agent/src/chat.rs:500`](../../../crates/foks-agent/src/chat.rs#L500): PendingChatStore shows the existing master-key-encrypted local store pattern

**Recommendation**

Add ChatAction::Search {channel, query, before?, page_budget} to the agent. It reads backward through read_chat_thread windows (verified, anchors updated), applies case-folded substring matching in memory, and returns hits (sequence, sender, send_time, snippet with match offsets) plus scanned_through so the desktop can continue. Cap each call at about 20 pages or 4 MiB, make it cancellable, and run it under the existing per-profile serialization. Cross-channel search iterates readable channels with a smaller per-channel budget. Phase 2, optional: an agent-owned index encrypted under the vault master key, following PendingChatStore, with per-channel coverage ranges, invalidated on key rotation and team removal.

**Mockup:** [Find in conversation and message search](../mockups/chat-find-and-search.html)

<details><summary>Verifier note</summary>

No message search exists. ChatAction (crates/foks-agent-proto/src/chat.rs:9) has History and NotificationHistory but nothing for search. The desktop search palette (apps/desktop/src/shell/search-palette.tsx) matches channel names only, and the chat-tab query (chat-tab.tsx:63-66, chat-teams.tsx:107-110) only filters the inbox list. CHAT_PAGE_ROWS is 50 (chat-limits.json:4). The agent's PreviewCache is in-memory only and never persisted (agent chat.rs:245-260), and PendingChatStore::open(state_dir, master) at chat.rs:500 is the existing master-key-encrypted store pattern. read_chat_thread (client-app chat.rs:292) and ChatSession::read_thread (history.rs:100) already give verified bounded windows of up to 1000 sequences and 8 MiB, so an agent-side backward scan needs no protocol change and does not touch the ciphertext-only server. One implementation note: network chat reads take the exclusive per-profile session (read_cache.rs:577-597), so the search should acquire it per window and release it between windows, so sends and inbox sync are not blocked for the whole 20-page budget. Low priority and M effort are reasonable.

</details>

### chat-backend-presence-typing-receipts

**Typing indicators and other members' read positions as capability-gated ephemeral extensions**

- Type: missing-feature
- Priority: low
- Effort: L
- Layers: protocol, server, agent, desktop-ui
- Verification: confirmed

The poll result is the Go shape {bumped, inbox_version}, and every inbox change is a durable write on the single writer, so typing cannot go through the inbox. The server already stores read_through for every member, but returns only the caller's own, and read-through changes wake only the reader. Both features need Rust-only methods. Neither needs any persisted plaintext.

**Evidence**

- [`crates/foks-proto/src/realtime.rs:518`](../../../crates/foks-proto/src/realtime.rs#L518): RtInboxPollResult carries only bumped and inbox_version
- [`crates/foks-server-db/src/schema/realtime.sql:45`](../../../crates/foks-server-db/src/schema/realtime.sql#L45): rt_user_channels stores read_through per user and channel
- [`crates/foks-server-db/src/realtime/inbox.rs:396`](../../../crates/foks-server-db/src/realtime/inbox.rs#L396): rt_read_through wakes only the reader
- [`crates/foks-server/src/net/session/handlers/realtime.rs:101`](../../../crates/foks-server/src/net/session/handlers/realtime.rs#L101): Poll loop keyed by user inbox listener

**Recommendation**

Behind a presence capability: (1) foksChatReadStates {channel} returns [(uid, read_through)] for current readers after require_read. Each user controls a share_read_state opt-in stored in rt_user_inboxes, off by default. (2) foksChatTyping {channel} records typing in an in-memory per-channel hub with about a 6 s TTL, rate-limited per user and never persisted. (3) foksChatPresencePoll {channel, since_epoch, timeout} long-polls that hub under the existing poll permit limit, and also returns read-state changes for the channel. The agent runs it only for the focused channel. Go clients and servers are unaffected.

**Mockup:** [Extended chat preview: replies, files, channel settings, presence](../mockups/chat-capability-gated-extensions.html)

<details><summary>Verifier note</summary>

RtInboxPollResult (crates/foks-proto/src/realtime.rs:518) is exactly {bumped, inbox_version}. rt_user_channels (realtime.sql:45-56) stores read_through for each user and channel. rt_read_through (inbox.rs:332-404) returns a wake target only for the actor's own uid, so other members never learn read positions. The poll handler (handlers/realtime.rs) builds its loop on a per-user inbox listener; the listener is created at line 108 and the loop starts at 118, not 101. A per-session realtime_polling semaphore exists (net/session.rs:163) and can bound the proposed presence poll. Rust-only methods prefixed 'foks' are an established pattern (foksChatCapabilities in generated routes), so Go clients and servers are unaffected. Nothing in the repository implements typing indicators or shared read state. The opt-in default and the no-persistence rule fit the ciphertext-only server constraint. The design is sound.

</details>
