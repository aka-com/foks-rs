# Local agent, IPC protocol, CLI and MCP

Area key `agent`. 15 verified findings. Part of the [foks-rs review](../README.md).

## Current state

The agent is a single bounded process that owns credentials, sessions and verification. Its internals are carefully built: the socket ownership and peer-UID checks, profile admission that answers before the client's deadline, the classification of ambiguous outcomes, per-response phase timing and the background-loop timers all hold up. The weaknesses are in its interfaces and its structure. IPC versioning is exact-equality with no handshake. The agent's own VersionMismatch reply cannot be decoded by a client of a different version, so the CLI and MCP show misleading "agent unavailable" errors. Error disposition (not executed / rejected / unknown, retryable) is classified only inside the desktop crate. The CLI drops error codes and always exits 1, and MCP collapses most errors into one string. Two correctness risks stand out. When connection capacity is full, the agent drops the socket without a reply, which turns unread mutations into ambiguous outcomes. DataRead::Catalog is unpaged, so once a store is large enough the response exceeds the 1 MiB frame and the agent closes the connection. Chat inbox polling and pending-operation polling run in the renderer, and the agent allows one poll per account, so the CLI and MCP cannot watch chat while the desktop is open, and change detection stops whenever the renderer is not running. Feature parity is uneven. The CLI has no chat commands, and it splits between in-process state access (most commands) and agent IPC (newer commands). MCP exposes only KV and team reads. Its only consent control is --read-only: there is no scope, audit trail or revocation, and it is not paused while the desktop is locked. Maintainability debt is concentrated in foks-agent/src/main.rs (9,756 lines, including a 2,196-line dispatch match), 13 process-global singletons, a binary-only crate (which causes the stale-binary test failures documented in AGENTS.md), eight separate operation classifiers that already disagree, and bot reply shapes defined three times across agent, Tauri and TypeScript. Observability is limited to unstructured eprintln output. That output is unrotated when the desktop launches the agent and discarded when the CLI launches it.

## Index

| Finding | Type | Priority | Effort |
|---|---|---|---|
| [Replace exact-equality IPC versioning with a frozen hello/capability handshake](#agent-ipc-version-handshake) | maintainability | high | M |
| [Define bot replies as typed proto structs, generate the TypeScript decoder, and report loaded state](#agent-bot-contract-typed) | maintainability | medium | M |
| [Answer connection-capacity rejections with a definite Busy instead of silently closing the socket](#agent-capacity-drop-false-ambiguity) | correctness-risk | medium | S |
| [Move chat inbox polling into the agent and expose a subscription stream to all front ends](#agent-chat-subscription-stream) | missing-feature | medium | L |
| [Add CLI chat commands and close remaining CLI parity gaps (team KV, mv/stat, pending work, agent status, completions)](#agent-cli-chat-and-parity) | missing-feature | medium | M |
| [Route the remaining direct-access CLI commands through the agent](#agent-cli-direct-state-access) | maintainability | medium | L |
| [Page DataRead::Catalog, add single-path lookup for MCP, and turn oversize responses into a typed error](#agent-data-catalog-unbounded) | correctness-risk | medium | M |
| [Define error outcome/retry/category in foks-agent-proto and use it for CLI exit codes and MCP tool errors](#agent-error-disposition) | correctness-risk | medium | M |
| [Split foks-agent into a library with domain dispatch modules and an explicit AgentContext](#agent-main-decomposition-lib) | maintainability | medium | L |
| [Expose a guarded MCP chat tool set on the existing chat operations](#agent-mcp-chat-tools) | missing-feature | medium | M |
| [Add agent-enforced assistant grants, an audit log, revocation and desktop-lock propagation for MCP sessions](#agent-mcp-consent-audit) | security | medium | L |
| [Give the agent its own bounded, timestamped log, counters and a Diagnostics operation](#agent-observability-logs-diagnostics) | missing-feature | medium | M |
| [Carry the client's deadline in each request instead of an agent-wide timeout set by whichever front end launched the agent](#agent-request-deadline) | correctness-risk | medium | S |
| [Write real MCP tool descriptions and correct the tool annotations](#agent-mcp-tool-metadata) | feature-refinement | low | S |
| [Snapshot every operation's policy row so the eight classifiers cannot drift apart](#agent-operation-policy-table) | testing | low | S |

### agent-ipc-version-handshake

**Replace exact-equality IPC versioning with a frozen hello/capability handshake**

- Type: maintainability
- Priority: high
- Effort: M
- Layers: protocol, agent, client-lib, cli, mcp, desktop-native, docs
- Verification: adjusted

Every IPC frame must carry exactly PROTOCOL_VERSION (33). The agent's VersionMismatch reply carries the agent's own version, so a mismatched client fails in decode_response with Error::Version and never learns the agent's version or supported range. After an upgrade that leaves an old resident agent running, the CLI's ensure_agent treats the failed Ping as 'no agent'. It spawns a sibling that loses the state-directory lock with stderr discarded, then tells the user to 'initialize and unlock' the state. MCP reports 'agent unavailable'. For a mutation, agent-client wraps the decode failure in Ambiguous, so a request the agent rejected unexecuted is reported as outcome-unknown, and in the desktop as ambiguous+fatal. AgentStatus.history_after is an ad-hoc capability flag that can never be absent under exact matching. book/21-mcp.qmd (32) and book/17-wire.qmd (30) both state stale versions.

**Evidence**

- [`crates/foks-agent-proto/src/frame.rs:89`](../../../crates/foks-agent-proto/src/frame.rs#L89): validate_version rejects any frame whose version != PROTOCOL_VERSION; decode_response (line 77) applies it to responses
- [`crates/foks-agent/src/main.rs:1053`](../../../crates/foks-agent/src/main.rs#L1053): On Error::Version the agent replies with Response::error_with_fields(VersionMismatch, ErrorFields::default()), stamped with its own PROTOCOL_VERSION (message.rs:2310), so a client of another version cannot decode the reply
- [`crates/foks-agent-client/src/lib.rs:176`](../../../crates/foks-agent-client/src/lib.rs#L176): call_platform wraps read/decode failures on mutations in Error::Ambiguous, so a version-mismatch reply to a mutation that was never executed is reported as an unknown outcome
- [`crates/foks-desktop/src/lib.rs:388`](../../../crates/foks-desktop/src/lib.rs#L388): Ambiguous(Protocol(Version)) becomes Ipc{code: VersionMismatch, ambiguous: true}
- [`crates/foks-agent-proto/src/message.rs:449`](../../../crates/foks-agent-proto/src/message.rs#L449): history_after: Option<bool> capability flag ('Older agents reject unknown fields'); the agent always sets Some(true) at main.rs:3407
- [`crates/foks-cli/src/mcp.rs:51`](../../../crates/foks-cli/src/mcp.rs#L51): ensure_agent: any Ping failure spawns a sibling agent with stderr nulled; the sibling loses the lock (main.rs:6226) and the CLI reports 'initialize and unlock the selected state first'
- [`crates/foks-mcp/src/agent.rs:542`](../../../crates/foks-mcp/src/agent.rs#L542): call() maps every non-Ambiguous client error, including Protocol(Version), to 'authenticated local agent unavailable'
- [`crates/foks-agent-proto/src/lib.rs:662`](../../../crates/foks-agent-proto/src/lib.rs#L662): tests hard-code assert_eq!(PROTOCOL_VERSION, 33) (also line 679)
- [`book/21-mcp.qmd:213`](../../../book/21-mcp.qmd#L213): 'its current version is 32' is stale
- [`book/17-wire.qmd:349`](../../../book/17-wire.qmd#L349): 'Protocol version | 30' is stale

**Recommendation**

1) In foks-agent-proto, add a Hello exchange with a frozen schema that does not depend on the version: client kind and version, an accepted [min,max] range, and a capabilities list. The agent answers Hello before validate_version. 2) Freeze the version-mismatch error envelope. decode_response returns a typed Error::VersionMismatch { agent, supported } for status=error and code=version-mismatch whatever the version, and the agent fills in the supported range. agent-client must not wrap this error in Ambiguous for mutations, because the agent never executed the request. 3) Move history_after into the Hello capabilities. 4) Optionally accept [N-1, N], but only if the agent also shapes each response to the negotiated version. Many proto types are deny_unknown_fields, so new response fields break an N-1 client. Without that per-version response handling, keep exact matching and rely on the clear diagnosis from steps 1, 2 and 5. 5) In ensure_agent and MCP call(), use Hello. On a mismatch, report 'agent speaks protocol X, this client speaks Y; restart the agent', and do not spawn a second agent. 6) Fix book/21-mcp.qmd:213 and book/17-wire.qmd:349. Make the tests reference PROTOCOL_VERSION symbolically.

<details><summary>Verifier note</summary>

The core claim holds. validate_version (frame.rs:89-95) requires version == PROTOCOL_VERSION and runs on requests, responses and upload frames. The agent's VersionMismatch reply (main.rs:1053-1066) is built by Response::error_with_fields, which stamps PROTOCOL_VERSION (message.rs:2310), so a client of another version fails in decode_response with Error::Version. ErrorFields has no field for a protocol range. In ensure_agent (mcp.rs:51-91), a failed Ping spawns a sibling agent with stderr discarded. The sibling fails try_lock_exclusive ('another agent owns this state directory', main.rs:6226), and ensure_agent then reports 'agent did not start; initialize and unlock the selected state first'. ensure_agent is called by bot_token, web_admin, sso, invitations, account_conveniences, account sync and mcp. MCP call() (agent.rs:542-551) maps every non-Ambiguous error to 'authenticated local agent unavailable'. history_after is always Some(true) at main.rs:3407. Tests pin 33 at lib.rs:662 and 679. book/21-mcp.qmd:213 says 32. Three corrections: (1) book/17-wire.qmd:349 is also stale; its table says 'Protocol version | 30'. (2) A further defect the finding omits: call_platform (agent-client lib.rs:176-190) wraps any read or decode failure on a mutation in Error::Ambiguous. A mutation sent to a mismatched agent, which the agent never decoded, therefore surfaces as an unknown outcome. The desktop maps it to Ipc{code: VersionMismatch, ambiguous: true} (foks-desktop lib.rs:388-406), and MCP reports 'submission outcome unknown'. (3) Recommendation step 4 (accept [N-1, N]) is under-specified. 31 proto types use deny_unknown_fields, so even 'additive' response fields would break an N-1 client unless the agent shapes each response to the negotiated version. AGENTS.md:34 mentions mismatches only as a build-staleness workaround, and ISSUES.md does not track a handshake.

</details>

### agent-bot-contract-typed

**Define bot replies as typed proto structs, generate the TypeScript decoder, and report loaded state**

- Type: maintainability
- Priority: medium
- Effort: M
- Layers: protocol, agent, desktop-native, desktop-ui, cli
- Verification: adjusted

The agent returns bot results as untyped JSON: json!({account_alias, loaded}) for Load and Unload, serialized foks_client_app::BotEnrollmentReport values for List, Prepare, Attempt, Status and Revoke, and {report, token} for Export. The enrollment row is declared twice more. The Tauri layer has its own deny_unknown_fields Enrollment struct with the 5-character name rule, six state strings and 160-row limit, though it reuses the proto BotAction validators for operation and device ids. bot-contract.ts repeats all of these as regexes and borrows its state list from rename-contract.ts's renameStates. The CLI prints the raw JSON. Loaded bot material lives only in the agent's in-memory map, and List does not say which bot is loaded. After an agent restart or version takeover, the user only finds out from a BotTokenLocked error on the next use.

**Evidence**

- [`crates/foks-agent/src/bot_token.rs:85`](../../../crates/foks-agent/src/bot_token.rs#L85): Load/Unload return json!({"account_alias","loaded"}) (also line 92); Export returns json!({report, token}) at 155
- [`crates/foks-agent/src/bot_token.rs:5`](../../../crates/foks-agent/src/bot_token.rs#L5): loaded bots kept in a process-global OnceLock map keyed by (credential store, account alias)
- [`crates/foks-client-app/src/bot_token.rs:16`](../../../crates/foks-client-app/src/bot_token.rs#L16): BotEnrollmentReport is the canonical serialized row; state is a String
- [`apps/desktop/src-tauri/src/commands/bot.rs:41`](../../../apps/desktop/src-tauri/src/commands/bot.rs#L41): Enrollment struct (deny_unknown_fields) and project() re-declare the name, state-list, role-length and 160-row rules; ids are checked by reusing proto BotAction validate (75-83)
- [`apps/desktop/src/bot-contract.ts:26`](../../../apps/desktop/src/bot-contract.ts#L26): decodeBotReply re-validates with regexes and reuses renameStates for bot states
- [`crates/foks-cli/src/bot_token.rs:188`](../../../crates/foks-cli/src/bot_token.rs#L188): CLI prints the raw JSON value (and report for exports)
- [`crates/foks-agent/src/main.rs:1905`](../../../crates/foks-agent/src/main.rs#L1905): BotTokenLocked is the only signal that a bot is not loaded
- [`apps/desktop/src/components/bot-panel.tsx:11`](../../../apps/desktop/src/components/bot-panel.tsx#L11): existing bot panel with enroll/load/revoke panes; shows no loaded state

**Recommendation**

In foks-agent-proto::bot, add BotEnrollment, a BotEnrollmentState enum of its own (not shared with rename), BotSelection { account_alias, loaded } and a BotReply enum, each with validate(). The agent serializes these types, and Tauri and the CLI deserialize them instead of re-declaring them. Generate the TypeScript types and decoders from the proto with ts-rs, or schemars plus a generated decoder, following the existing chat-limits.json sharing (proto build.rs; apps/desktop/src/chat-limits.ts:8), and delete the hand-written decodeBotReply. Apply the same method to the rename, sso and invitation contracts. Add loaded: bool to each List row, computed from the agent's session map. When the desktop reconnects to a new agent pid, it compares against the previous loaded set and shows a notice.

<details><summary>Verifier note</summary>

Core confirmed. bot_token.rs returns json!({account_alias, loaded}) for Load and Unload (85, 92), json!({report, token}) for Export (155), and serialized foks_client_app::BotEnrollmentReport values elsewhere. Loaded material lives in a process-global OnceLock map (5-7). The Tauri Enrollment struct and project() (commands/bot.rs:41-99) re-declare the 5-character name rule, the six state strings, the 160-row limit and the 64-byte role bound. bot-contract.ts:26-73 repeats regexes, including /^13[0-9a-f]{64}$/, and reuses renameStates. The CLI prints raw JSON (bot_token.rs:188-190). List rows carry no loaded flag, and BotTokenLocked (main.rs:1905) is the only signal. ISSUES.md does not track this. Corrections: Tauri does not re-declare the operation-id and device-id rules; it reuses foks_agent_proto::bot::BotAction::{Cancel,Revoke}::validate (commands/bot.rs:75-83). The canonical row is foks_client_app::bot_token::BotEnrollmentReport (client-app bot_token.rs:16), with state as a String. The chat-limits.json mechanism shares constants, not types, so ts-rs or schemars would be a new dependency rather than an extension of it. A bot panel already exists (components/bot-panel.tsx, 410 lines, with Load/Unload/Revoke panes), so the mockup refines it rather than adding a new panel. Adding loaded to rows must ship in lockstep with a protocol version bump, because the Tauri struct is deny_unknown_fields and the TS decoder requires an exact key count. Proto must not depend on client-app, so the agent converts BotEnrollmentReport into the proto type.

</details>

### agent-capacity-drop-false-ambiguity

**Answer connection-capacity rejections with a definite Busy instead of silently closing the socket**

- Type: correctness-risk
- Priority: medium
- Effort: S
- Layers: agent, client-lib
- Verification: adjusted

When the 32 connection permits are exhausted, the accept loop drops the stream without reading, replying or logging. Clients write their frame immediately after connect, so a mutation the agent never read is reported as Ambiguous (outcome unknown), which the admission-margin design exists to prevent. A read is classified as connection loss, so the desktop transport fires its connection-lost notifier for a merely saturated agent. Saturation requires heavy fan-out, because chat polls are capped at one per account and MCP at 4 active calls. When it does occur, both misclassifications follow.

**Evidence**

- [`crates/foks-agent/src/main.rs:329`](../../../crates/foks-agent/src/main.rs#L329): let Ok(permit) = active.clone().try_acquire_owned() else { drop(stream); continue; } with no reply and no log; default maximum_connections 32 (line 158)
- [`crates/foks-agent-client/src/lib.rs:176`](../../../crates/foks-agent-client/src/lib.rs#L176): write or read failure on a mutation becomes Error::Ambiguous; on reads, EOF/reset counts as connection loss (lines 50-63)
- [`apps/desktop/src-tauri/src/agent/transport.rs:391`](../../../apps/desktop/src-tauri/src/agent/transport.rs#L391): call_observed issues one AgentClient::call_cancellable per request; record() (line 341) fires the connection-loss notifier on connection_lost errors
- [`apps/desktop/src/chat/inbox-service.ts:989`](../../../apps/desktop/src/chat/inbox-service.ts#L989): each account poll holds an agent connection for up to 25 s

**Recommendation**

When capacity is exhausted, hand the stream to a small bounded reject task with its own semaphore of, for example, 8 permits. The task reads the first frame with a 250 ms deadline, extracts the id with foks_agent_proto::request_id, writes Response::error_with_fields(id, Busy, "agent connection capacity reached", reason "admission-not-started"), and closes. If the reject semaphore is also full, or the frame does not arrive in time, close as today. Count both paths in a capacity_rejections counter (see the observability finding). Add a test that fills capacity with idle connections, sends a mutation, and asserts a typed Busy rather than Ambiguous. The chat subscription finding removes the long-held poll connections that make saturation likely.

<details><summary>Verifier note</summary>

The core claim holds. In the accept loop (main.rs:329-332), when try_acquire_owned fails the code drops the stream with no reply and no log, unlike the peer-credential rejections just above it, which do log. maximum_connections defaults to 32 (main.rs:158). call_platform (agent-client lib.rs:176-190) wraps any write or read failure on a mutation as Ambiguous. The client writes its frame immediately after connect, so a dropped connection yields EOF or ECONNRESET. The Tauri transport (src-tauri/agent/transport.rs:391-410) calls AgentClient::call_cancellable once per request and applies no client-side concurrency cap. The renderer's PollInbox uses timeout_milliseconds 25_000 (inbox-service.ts:989). The recommendation is sound and consistent with the existing admission-not-started contract, which the TS failureOutcome already recognizes. The finding omits one consequence: for reads, the dropped connection is classified as connection loss (is_connection_loss covers UnexpectedEof and ConnectionReset), so the desktop transport's record() fires the connection-loss notifier and the UI treats a saturated agent as a lost one. Priority is reduced to medium. Reaching 32 concurrent connections needs substantial fan-out: chat long polls are capped at one per (profile, account) by the active_chat_polls registry, and MCP is capped at 4 active calls. The defect is real, but it is not a common path.

</details>

### agent-chat-subscription-stream

**Move chat inbox polling into the agent and expose a subscription stream to all front ends**

- Type: missing-feature
- Priority: medium
- Effort: L
- Layers: agent, protocol, client-lib, desktop-ui, cli, mcp, docs
- Verification: adjusted

The agent answers PollInbox once per request and allows one active poll per (profile, account); a second caller gets Busy. The poll loop, backoff and reset handling live in the renderer, so change detection stops whenever the renderer stops. This blocks background notifications and contradicts book 19, which lists chat long-polls as agent background work. Each long poll also holds one of the 32 agent connections. Send recovery issues Pending/CleanupPending per team about every 30 s when idle and every 2 s while work is outstanding. Moving polling into the agent behind a subscription stream would remove the renderer polling and the single-consumer restriction, and would provide a basis for future CLI or MCP chat surfaces, which do not exist today.

**Evidence**

- [`crates/foks-agent/src/chat_poll.rs:86`](../../../crates/foks-agent/src/chat_poll.rs#L86): second poller for the same (profile, account) is refused with Busy 'chat synchronization is already active for this account'
- [`apps/desktop/src/chat/inbox-service.ts:972`](../../../apps/desktop/src/chat/inbox-service.ts#L972): renderer-owned poll() loop with 25 s poll-inbox requests (line 989), backoff and reset handling
- [`apps/desktop/src/chat/send-service.ts:151`](../../../apps/desktop/src/chat/send-service.ts#L151): IDLE_TICK = 30_000: Pending/CleanupPending recovery runs about every 30 s per team when idle and at the 2 s cadence (line 1254/1482) only while a team has outstanding work
- [`crates/foks-agent/src/read_cache.rs:527`](../../../crates/foks-agent/src/read_cache.rs#L527): stale comment claims the renderer issues Pending and CleanupPending every two seconds per team even when nothing is pending
- [`crates/foks-agent/src/timers.rs:13`](../../../crates/foks-agent/src/timers.rs#L13): agent background timers are retention, scheduler, compatibility and ownership; no chat polling
- [`book/19-agent.qmd:33`](../../../book/19-agent.qmd#L33): lists chat long-polls among agent background work, which the agent does not perform

**Recommendation**

Keep the phased design. Phase 1: Operation::Subscribe with ChatInbox and ChatOperations topics; one reference-counted agent poll task per subscribed account, reusing run_chat_poll and the chat_polling semaphore; bounded per-subscriber queues that collapse to Resync on overflow; the feature advertised as a Hello capability; AgentClient::subscribe(). Phase 2: emit OperationChanged from agent chat operation transitions so that the desktop can drop its Pending/CleanupPending polling, which costs about 30 s per team idle and 2 s while work is outstanding. Phase 3: switch the desktop inbox-service to events. Any CLI or MCP chat surface is new work rather than a migration, since neither front end has chat today. Phase 4 (with the deferred background work): agent-held subscriptions drive native notifications. Also correct book/19-agent.qmd:33 now, or implement the agent-side polling it describes, and fix the stale read_cache.rs comment.

**Already tracked:** ISSUES.md 'Existing disclosed limitations': reliable minimized/background operation and non-macOS native notifications remain deferred. This finding adds the agent-side subscription design and a first step that does not depend on that work.

<details><summary>Verifier note</summary>

Several claims are confirmed. The agent answers PollInbox per request and refuses a second poller for the same (profile, account) with Busy (chat_poll.rs:76-91; ChatPollKey holds profile and account). The poll loop, backoff and reset handling live in the renderer (inbox-service.ts:972-1010, 25 s requests). The agent runs no chat timer; its timers are retention, scheduler, compatibility and ownership. book/19-agent.qmd:33 nonetheless lists chat long-polls as agent background work, which is inaccurate. Each long poll holds one connection permit. ISSUES.md:185-186 tracks deferred background operation and notifications, and the finding adds a concrete agent-side design. Corrections: (1) Consequence (c) is wrong. send-service.ts has IDLE_TICK = 30_000 (line 151), and the tick runs at the 2 s ACTIVE_TICK/due cadence only while outstanding(team) is true. An idle team therefore costs Pending and CleanupPending about every 30 s, not every 2 s. The read_cache.rs:521-527 comment that says 'every two seconds per team even when nothing is pending' is itself stale. recovery-schedule.ts:75 schedules per-operation status checks for unobserved operations, not idle per-team polling. (2) Consequence (a) is hypothetical. Neither foks-cli nor foks-mcp has any chat command or tool (no Operation::Chat use), so 'chat watch' and 'MCP chat tools' would be new features rather than existing consumers that are blocked. The subscription design is sound and feasible: the agent owns credentials, and the upstream PollInbox RPC and Go interop are unchanged. Priority is medium, since the desktop is today the only chat consumer and the idle cost is lower than claimed.

</details>

### agent-cli-chat-and-parity

**Add CLI chat commands and close remaining CLI parity gaps (team KV, mv/stat, pending work, agent status, completions)**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: cli
- Verification: adjusted

The CLI has no chat subcommand, although the agent already exposes Channels, Inbox, History, SubmitMessage (a one-shot send bound to a durable 16-byte submission id), Status and Reconcile. KV commands cover only the account store (no --team), have no mv, stat or symlink, and get requires --output, so content cannot go to stdout. No command lists pending operations, describes server status, or shows agent status. --state-dir must come before the subcommand, has no environment default, and the binaries have no --version or shell completion.

**Evidence**

- [`crates/foks-cli/src/main.rs:41`](../../../crates/foks-cli/src/main.rs#L41): Command enum has no Chat variant; no foks-cli module references chat
- [`crates/foks-agent-proto/src/chat.rs:73`](../../../crates/foks-agent-proto/src/chat.rs#L73): ChatAction::SubmitMessage { submission, channel, text }; Channels/History/Inbox/Status/Reconcile/Pending are in the same enum, and Pending takes no operation id
- [`crates/foks-cli/src/main.rs:172`](../../../crates/foks-cli/src/main.rs#L172): KvCommand: List/Get/Put/Mkdir/Remove, account alias only; Get requires --output PathBuf
- [`crates/foks-cli/src/main.rs:33`](../../../crates/foks-cli/src/main.rs#L33): state_dir: PathBuf under #[arg(long)], with no global or env; #[command] at line 29 has no version
- [`crates/foks-agent-proto/src/message.rs:902`](../../../crates/foks-agent-proto/src/message.rs#L902): ListTeamKv, ReadKvChunk, PutKvSymlink and MoveKv exist; AgentStatus, ListPendingOperations and DescribeServerStatus are at lines 576, 634 and 658
- [`Cargo.toml:80`](../../../Cargo.toml#L80): clap enables only the derive feature; env = ... needs the env feature
- [`crates/foks-cli/README.md`](../../../crates/foks-cli/README.md): 'Output destinations for KV downloads ... must be new private files' is a documented, deliberate rule

**Recommendation**

Add 'foks-rs chat', routed through the agent, for named teams only. The subcommands are: channels <profile> --account-alias A --team T; inbox; history --channel <id|name> [--before SEQ]; and send --channel C (--text T | --stdin) [--submission HEX32]. send calls SubmitMessage, prints the operation id and state, and exits with a distinct code when the state is uncertain. Also add status --operation ID, reconcile --operation ID, and pending (no arguments). Resolve team_id with ListTeams and channel names with Channels. Document that a long-poll watch shares the single per-account poll slot with the desktop until a subscription stream exists. For KV, add --team (ListTeamKv and KvStoreRef::Team), 'kv mv' (MoveKv), 'kv stat' (ReadData/DataRead::Stat), 'kv ln' (PutKvSymlink), and an explicit opt-in 'kv get --output -' that streams via ReadKvChunk, documented as a deliberate exception to the README's new-private-file rule. Add 'foks-rs agent status|pending|server-status' using AgentStatus, ListPendingOperations and DescribeServerStatus. Add 'foks-rs completions <shell>' via clap_complete and #[command(version)] on foks-rs and foks-agent. Make state_dir `Option<PathBuf>` with #[arg(long, env = "FOKS_STATE_DIR", global = true)] and return a runtime error when it is missing. clap rejects required global arguments. Enable clap's "env" feature in the workspace.

<details><summary>Verifier note</summary>

The core claims hold. The Command enum (main.rs:41) has no Chat variant, and no CLI module mentions chat. KvCommand (main.rs:172) has only List/Get/Put/Mkdir/Remove for the account alias, with no team, mv, stat or symlink, and Get needs --output PathBuf. state_dir (main.rs:32-33) is #[arg(long)] with no global or env. Neither foks-rs nor foks-agent sets #[command(version)], and clap_complete is not used. The agent operations the finding names all exist: ListTeamKv, MoveKv, PutKvSymlink, ReadKvChunk, AgentStatus, ListPendingOperations, DescribeServerStatus, and ReadData/DataRead::Stat for stat. The PollInbox conflict is real: the agent allows one active poll per (profile, account) (chat_poll.rs ChatPollKey). ISSUES.md does not track any of this. Four parts of the recommendation need correction. (1) The exact attribute `#[arg(long, env = "FOKS_STATE_DIR", global = true)] state_dir: PathBuf` fails clap's debug assertion 'Global arguments cannot be required' (clap_builder 4.6.6 debug_asserts.rs:268-273), so debug builds and tests panic. The field must become Option<PathBuf> with a runtime error when it is absent, and the workspace clap dependency needs the "env" feature, since it enables only "derive" (Cargo.toml:80). (2) The foks-cli README deliberately requires KV download destinations to be new private files, and kv get enforces create_new with mode 0600, so `--output -` loosens a documented security rule. It must be an explicit opt-in and the change must be documented. (3) ChatAction::Pending takes no operation id; only Status and Reconcile take `operation`. (4) Chat calls need a TeamStoreRef with team_id, so the CLI has to resolve it, for example via ListTeams. Basic chat supports only named teams with direct same-host membership (ISSUES.md). The SubmitMessage evidence line is 73, not 69.

</details>

### agent-cli-direct-state-access

**Route the remaining direct-access CLI commands through the agent**

- Type: maintainability
- Priority: medium
- Effort: L
- Layers: cli, agent, docs
- Verification: adjusted

Most CLI command groups (profile, account list/create/resume, kv, jobs, device, recovery, passphrase, team except invite, yubi) open the registry, decrypt the vault and run client-app operations in-process. Newer groups (bot, admin, rename, sso, invitations, account sync, retention status) call the agent. The state-root lease is shared, so direct commands run alongside a resident agent. Per-operation exclusive profile and database locks serialize the actual mutations, but account keys are decrypted in a second process, contrary to the agent-owns-credentials model. CLI writes do not invalidate the agent's catalog cache, which can serve a first page up to 60 s old. Every feature needs two implementations with different output formatting. Book 19 says the CLI, desktop and MCP all use the same socket, which is inaccurate. The foks-cli README describes the direct design as intentional.

**Evidence**

- [`crates/foks-cli/src/main.rs:1045`](../../../crates/foks-cli/src/main.rs#L1045): kv_command opens ProfileRegistry/ProfileSession and with_vault in-process
- [`crates/foks-cli/src/main.rs:948`](../../../crates/foks-cli/src/main.rs#L948): account sync goes through ensure_agent and AgentClient
- [`crates/foks-client-app/src/portability/lease.rs:53`](../../../crates/foks-client-app/src/portability/lease.rs#L53): ClientStatePathLease takes a shared lock, as does the namespace lock at line 105. ProfileRegistry::open acquires the lease (registry.rs:662), so a direct CLI and the agent coexist
- [`crates/foks-client-app/src/checkpoint.rs:729`](../../../crates/foks-client-app/src/checkpoint.rs#L729): each checked operation holds an exclusive ProfileLock and DatabaseLock across processes
- [`crates/foks-agent/src/main.rs:2608`](../../../crates/foks-agent/src/main.rs#L2608): may_serve_cached_catalog serves the first page from a cache with CATALOG_CACHE_LIFETIME = 60 s (line 101) unless fresh is set
- [`book/19-agent.qmd:284`](../../../book/19-agent.qmd#L284): 'One agent, three front ends' says the CLI uses the same socket; most CLI commands do not
- [`crates/foks-cli/README.md:3`](../../../crates/foks-cli/README.md#L3): 'foks-rs is the direct, non-interactive standalone client' documents the current design
- [`crates/foks-agent/src/main.rs:6188`](../../../crates/foks-agent/src/main.rs#L6188): AgentLock (exclusive .foks-rs.lock) is private to the agent binary

**Recommendation**

First, correct book 19 and the foks-cli README to state which CLI groups use the agent and which open state in-process. Then migrate one command group at a time with existing operations. Start with kv (ListKv/ListTeamKv, ReadKvChunk, PutKvStream, MkdirKv, RemoveKv, MoveKv). Then team (ListTeams, AddTeamMember, Promote/Demote/RemoveTeamMember, Resume*), then device and recovery (ListDevices, RemoveDevice, PrepareOwnerBackup/CommitOwnerBackup, RecoverOwnerAccount), then yubi and passphrase. Keep in-process access only for init, profile add/remove/reset-hard-state, state maintenance, and an explicit --offline mode. For --offline, move AgentLock into a shared crate so that mode takes the exclusive .foks-rs.lock and cannot run beside an agent. Put the call, error mapping and output formatting in one cli::agent module. Record the design change from 'direct standalone client' to 'agent client with an offline mode' in the README.

<details><summary>Verifier note</summary>

The core claim holds. kv, profile, account list/create/resume, jobs, device, recovery, passphrase, team (except invite) and yubi call ProfileRegistry::open, ClientCredentials and with_vault in-process. bot, admin, rename, sso, invitations, account sync and the retention status read use AgentClient. Book 19 line 284 says all three front ends use the same socket, which is inaccurate for most CLI commands. The agent's catalog cache serves first pages for up to 60 s unless fresh is set (main.rs:101 and 2608). Four corrections. (1) lease.rs:218 is the generic try_lock helper. The shared locks are taken at lease.rs:53 (path) and lease.rs:105 (namespace), and ProfileRegistry::open acquires the lease (registry.rs:662). (2) The concurrency risk is overstated. Every checked operation in either process holds an exclusive ProfileLock and DatabaseLock and re-verifies the rollback checkpoint (checkpoint.rs:729-731), so journal advances are serialized by more than one file lock. (3) Writes from other devices make the catalog cache stale in the same way, so the cache point is not specific to the CLI. Other stale read-cache entries only cause a failed read, which is retried (book 19). (4) The foks-cli README says 'foks-rs is the direct, non-interactive standalone client', so direct access is documented design. The finding stands because of the project constraint that the agent owns credentials, the duplicated implementations, and the book's inaccuracy, but it should name the README position it changes. The migration is feasible: the agent operations exist and the PIN and passphrase fields are SecretString. AgentLock lives in the foks-agent binary (main.rs:6188), so --offline needs it moved into a shared crate.

</details>

### agent-data-catalog-unbounded

**Page DataRead::Catalog, add single-path lookup for MCP, and turn oversize responses into a typed error**

- Type: correctness-risk
- Priority: medium
- Effort: M
- Layers: agent, protocol, mcp, docs
- Verification: adjusted

MCP list, get and stat each depend on a full DataRead::Catalog read. The agent refuses that read beyond 1,000 entries ('use the file client'), so MCP cannot operate on larger stores, even though the agent already has a paged, snapshot-bound catalog for the desktop. Get reads the full catalog twice and resolves paths by linear scan, and PAGE_ROWS is unused. Separately, write_response turns an oversized response into a silent connection close (Ambiguous for a mutation). This is reachable within the 1,000-row cap when paths are long (up to 4096 bytes).

**Evidence**

- [`crates/foks-client-app/src/kv/data.rs:366`](../../../crates/foks-client-app/src/kv/data.rs#L366): data_catalog refuses more than 1,000 entries ('catalog exceeds bounded adapter limit; use the file client'); data_members refuses rosters over 1,000 at line 31
- [`crates/foks-agent/src/data.rs:177`](../../../crates/foks-agent/src/data.rs#L177): DataRead::Catalog serializes all entries (at most 1,000) in one response
- [`crates/foks-agent-proto/src/data.rs:64`](../../../crates/foks-agent-proto/src/data.rs#L64): DataCatalog { entries: Vec<DataCatalogEntry> } with no cursor
- [`crates/foks-agent/src/main.rs:6133`](../../../crates/foks-agent/src/main.rs#L6133): write_response: foks_agent_proto::encode(response)? returns Err on TooLarge, so the connection closes without a reply; reachable with 1,000 long paths (up to 4096 bytes each)
- [`crates/foks-mcp/src/agent.rs:270`](../../../crates/foks-mcp/src/agent.rs#L270): get reads the full catalog, then again at line 319 to revalidate; list (227) and stat (334) also read the full catalog; resolve() scans linearly per component (131)
- [`crates/foks-mcp/src/contract.rs:12`](../../../crates/foks-mcp/src/contract.rs#L12): PAGE_ROWS declared and unused

**Recommendation**

1) In write_response, when encode returns TooLarge, send Response::error_with_fields(id, OperationFailed, 'response exceeds the IPC frame limit', reason 'response-too-large') instead of closing. 2) Add a paged DataRead::CatalogPage { cursor, limit } that reuses the snapshot-bound cursor and catalog cache the desktop already uses, so MCP list can serve stores with more than 1,000 entries and report truncation using PAGE_ROWS. 3) Extend DataRead::Stat, or add a sibling operation, so that it follows symlinks inside the agent and returns the entry plus snapshot_version. MCP get and stat then need no full-catalog read, and get revalidates by comparing snapshot_version and the entry rather than by a second catalog read. 4) Page Members in the same way if larger rosters are to be supported. 5) Add an agent test that exercises the TooLarge reply with long paths, and an MCP test that lists past the first page.

<details><summary>Verifier note</summary>

The headline failure mode is wrong. data_catalog in foks-client-app/src/kv/data.rs:366 refuses catalogs larger than 1,000 entries with 'catalog exceeds bounded adapter limit; use the file client'. data_members (data.rs:31) likewise refuses rosters larger than 1,000. A store of a few thousand entries therefore produces a typed agent error, which MCP shows as '<code>: catalog exceeds ...', not a silent close reported as 'authenticated local agent unavailable'. DataRead::Members is bounded too, by refusal. The bound in book 21 ('1,000 rows per page') is enforced as a hard refusal rather than through PAGE_ROWS. What remains true: (1) MCP list, get and stat all depend on a full-catalog read (agent.rs:227, 270, 334). Get reads the catalog a second time to revalidate (line 319), and resolve() scans linearly for each path component (line 131). MCP cannot operate at all on a store with more than 1,000 entries, even though the agent already has a snapshot-bound paged catalog (KvPage.next_cursor) for the desktop. (2) PAGE_ROWS (contract.rs:12) is unused. (3) write_response (main.rs:6133-6143) propagates encode's TooLarge and closes without a reply. Within the 1,000-entry cap this is still reachable, because paths may be up to 4096 bytes (data_write.rs:570): about 750 bytes of average path across 1,000 rows exceeds the 1 MiB frame. Any other oversized success reply hits the same path and becomes Ambiguous for a mutation. DataRead::Stat { path, version } already performs a path-only walk (data.rs:392-397), so the proposed Lookup should extend Stat with symlink following rather than add a new operation. Priority is medium: the defect is a capability limit with a clear error, not a silent failure.

</details>

### agent-error-disposition

**Define error outcome/retry/category in foks-agent-proto and use it for CLI exit codes and MCP tool errors**

- Type: correctness-risk
- Priority: medium
- Effort: M
- Layers: protocol, agent, client-lib, cli, mcp, desktop-native
- Verification: adjusted

ErrorCode is a flat 49-variant enum. Whether a failure means not-started, rejected or outcome-unknown is encoded ad hoc: reason='admission-not-started', a Rust desktop table (foks-desktop/src/lib.rs), and two separate TypeScript tables (operation-outcome.ts, bridge/errors.ts). The copies already disagree on Busy. Seven CLI agent-backed commands drop the code, and all CLI failures exit 1 without JSON error output. MCP protocol-level errors are text-only, though write outcomes are already structured. Scripts and assistants cannot tell conflict, busy-before-start and outcome-unknown apart.

**Evidence**

- [`crates/foks-agent-proto/src/message.rs:2359`](../../../crates/foks-agent-proto/src/message.rs#L2359): ErrorCode: flat enum of 49 variants with no outcome/retry metadata
- [`crates/foks-desktop/src/lib.rs:89`](../../../crates/foks-desktop/src/lib.rs#L89): retryable/security_failure on IpcErrorCode and transient/fatal/ambiguous on AgentError (100-160) exist only in the desktop crate; Busy is transient and not ambiguous
- [`apps/desktop/src/operation-outcome.ts:27`](../../../apps/desktop/src/operation-outcome.ts#L27): failureOutcome duplicates the classification in TS and treats Busy/ProfileBusy without admission-not-started as 'unknown', which disagrees with the Rust desktop
- [`apps/desktop/src/bridge/errors.ts:85`](../../../apps/desktop/src/bridge/errors.ts#L85): normalizeMutationError/commandRecovery is a further TS copy of the classification
- [`crates/foks-agent/src/main.rs:1858`](../../../crates/foks-agent/src/main.rs#L1858): dispatch_error_response: only AdmissionError gets reason 'admission-not-started'
- [`crates/foks-cli/src/invitations.rs:360`](../../../crates/foks-cli/src/invitations.rs#L360): ResponseResult::Error { message, .. } drops the code; also bot_token.rs:194, web_admin.rs:60, sso.rs:213, account_conveniences.rs:66/117, main.rs:962 (retention.rs:37 keeps it via Debug)
- [`crates/foks-cli/src/main.rs:702`](../../../crates/foks-cli/src/main.rs#L702): std::process::exit(1) for every error
- [`crates/foks-mcp/src/agent.rs:554`](../../../crates/foks-mcp/src/agent.rs#L554): protocol-level tool errors are format!("{code}: {message}") strings; write outcomes are already structured in agent/writes.rs:153

**Recommendation**

In foks-agent-proto, add exhaustive per-code defaults for category, retry and outcome. Outcome must default to Unknown for codes the agent can emit after execution has begun, including Busy, ProfileBusy and DeadlineExceeded. Add an optional ErrorFields.outcome, or reuse reason, that the agent sets when it knows the request did not start, as at admission and at the capacity reject. Give foks_agent_client::Error the same methods, and never mark a version-mismatch rejection as Ambiguous. Replace the Rust desktop tables with these methods, and have the native layer forward the computed outcome so that operation-outcome.ts and bridge/errors.ts stop re-deriving it. In the CLI, one agent-call helper replaces the 7 code-dropping matches and the 10 "foks-rs.sock" joins. It derives the exit code from outcome first (outcome unknown gets a distinct code regardless of category), then from category. With --json, it prints {"error":{code,message,fields,outcome,retry}}. In MCP, protocol-level tool errors carry structuredContent {code, outcome, retry}, matching the structured write outcomes that already exist.

<details><summary>Verifier note</summary>

The core claim holds. ErrorCode (message.rs:2359) is a flat enum of 49 variants with no outcome or retry metadata. reason='admission-not-started' is set only in dispatch_error_response (main.rs:1867). The classification methods retryable, security_failure, transient, fatal and ambiguous exist only in crates/foks-desktop/src/lib.rs (89-160). Seven CLI sites drop the code with ResponseResult::Error { message, .. }: invitations.rs:360, bot_token.rs:194, web_admin.rs:60, sso.rs:213, account_conveniences.rs:66 and 117, and main.rs:962. main.rs:702 always calls exit(1). MCP call() returns only the string '{code}: {message}' (agent.rs:554). No ISSUES.md or book entry tracks this. Corrections: (1) 'every agent-backed command' discards the code is overstated; retention.rs:37 keeps it, formatted with Debug. (2) There are 10 hard-coded "foks-rs.sock" joins in foks-cli, not 9. The agent has DEFAULT_SOCKET_NAME (main.rs:56), but no shared crate exports it. (3) Classification is duplicated a third time in TypeScript: apps/desktop/src/operation-outcome.ts failureOutcome and bridge/errors.ts commandRecovery. The copies already disagree. The Rust desktop treats Busy as transient and not ambiguous. The TS treats Busy or ProfileBusy without reason=admission-not-started as 'unknown', because 'a callback can itself report contention after execution has begun'. This supports a single source, but it also means outcome() cannot be a static per-code table. Busy and ProfileBusy must default to Unknown unless the agent marks the error not-started. The recommended CLI mapping that puts busy in exit code 5 ('busy/rate-limited') would wrongly tell scripts that a post-execution Busy is safe to retry. (4) MCP writes already return a structured DataWriteStatus, including SubmissionUnknown, with structured_content and status guidance (agent/writes.rs:153-172). Ambiguous IPC already produces text telling the model to 'inspect status'. The gap is confined to protocol-level errors such as DeadlineExceeded and Busy. Priority is medium: the defect lowers the quality of the scripting and assistant interfaces, but the desktop already handles these outcomes.

</details>

### agent-main-decomposition-lib

**Split foks-agent into a library with domain dispatch modules and an explicit AgentContext**

- Type: maintainability
- Priority: medium
- Effort: L
- Layers: agent, tooling, ci
- Verification: confirmed

main.rs is 9,756 lines. It holds dispatch_result_inner (a 2,196-line match over about 105 operations), the error mapping (about 560 lines), the KV catalog cache and cursor codec (about 560 lines), the scheduler and compatibility-lease loop, socket ownership, and 3,400 lines of tests. Agent state lives in 13 process-global OnceLock singletons spread across modules. The crate is binary-only, so CLI integration tests spawn a sibling target/debug/foks-agent. AGENTS.md documents that this binary goes stale and can need a forced relink to clear spurious version-mismatch failures.

**Evidence**

- [`crates/foks-agent/src/main.rs:3358`](../../../crates/foks-agent/src/main.rs#L3358): dispatch_result_inner spans 3358-5554
- [`crates/foks-agent/src/main.rs:2480`](../../../crates/foks-agent/src/main.rs#L2480): catalog cache, cursor encode/decode and pagination through 2997
- [`crates/foks-agent/src/main.rs:6350`](../../../crates/foks-agent/src/main.rs#L6350): #[cfg(test)] mod tests from 6350 to 9756
- [`crates/foks-agent/src/bot_token.rs:6`](../../../crates/foks-agent/src/bot_token.rs#L6): static SESSIONS OnceLock (one of 13 statics: chat.rs:305/382, connectivity.rs:68-69, main.rs:119/2583, profile_work.rs:133, read_cache.rs:137/147/348, retention.rs:89, timers.rs:55)
- [`crates/foks-cli/tests/bot_ipc.rs:86`](../../../crates/foks-cli/tests/bot_ipc.rs#L86): tests exec the sibling foks-agent binary and assert it exists
- [`AGENTS.md:34`](../../../AGENTS.md#L34): documented stale-agent relink workaround for IPC version mismatch in CLI tests

**Recommendation**

Add src/lib.rs exposing AgentConfig and an async serve(config, listener, shutdown); main.rs keeps argument parsing and the runtime. Move code into these modules: ipc/connection.rs (handle_connection, supervise_blocking, frame IO), ipc/errors.rs (dispatch_error_response, client_error_response, remote_status_response), ownership.rs (AgentLock, SocketGuard, bind and stale-socket handling), background/{scheduler,compatibility}.rs, kv/{dispatch,catalog}.rs, and dispatch/{profiles,accounts,devices,yubi,teams,federation}.rs. Each dispatch module exposes pub(crate) fn dispatch(ctx: &AgentContext, op) -> Result<Value>, so dispatch_result_inner becomes a short router by operation family. Introduce AgentContext, holding state_dir, caches, coordinator, timers, bot sessions and the preview cache, and replace the statics one module at a time, starting with the catalog cache and reset tickets in main.rs. Move tests alongside their modules. Then CLI and desktop integration tests start an in-process agent on a temporary socket through foks_agent::serve, keeping one smoke test that execs the real binary.

<details><summary>Verifier note</summary>

Every quantitative claim checks out. main.rs is 9,756 lines. dispatch_result_inner starts at 3358 and ends at about 5552, a match over the 107-variant Operation enum. The error mapping spans 1858 to about 2420 (dispatch_error_response, client_error_response, remote_status_response and trust helpers). The catalog binding, cache and cursor code spans about 2479-2997. The last #[cfg(test)] mod tests runs from 6350 to EOF. There are exactly 13 non-test OnceLock statics at the cited locations (bot_token.rs:6, chat.rs:305/382, connectivity.rs:68-69, main.rs:119/2583, profile_work.rs:133, read_cache.rs:137/147/348, retention.rs:89, timers.rs:55). Cargo.toml declares only a [[bin]] target. bot_ipc.rs:86-87 execs the sibling binary and asserts it exists, and AGENTS.md:34-38 documents the stale-binary relink workaround. ISSUES.md does not track this. The recommendation is feasible. It touches neither the wire nor server behaviour, and its ordering correctly replaces the statics before tests run in-process agents, because multiple in-process agents would otherwise share the global caches.

</details>

### agent-mcp-chat-tools

**Expose a guarded MCP chat tool set on the existing chat operations**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: mcp, cli, docs
- Verification: adjusted

The MCP server offers only KV and team-roster tools, so an assistant cannot summarize unread channels or post a message. The agent already provides scope-bound reads (Channels, Inbox, History in 50-row pages) and an idempotent send: SubmitMessage takes a client-chosen submission id and pairs with Status/Reconcile, the same exactly-once pattern MCP KV writes use. Chat content is written by other team members, so it is untrusted input to the assistant, and sending posts to other people. Both need explicit controls.

**Evidence**

- [`crates/foks-mcp/src/contract.rs:166`](../../../crates/foks-mcp/src/contract.rs#L166): tool sets: Team {list, list-memberships}, Kv {list, get, stat, usage, put, mkdir, rm, mv, foks_status, foks_pending}
- [`crates/foks-agent-proto/src/chat.rs:73`](../../../crates/foks-agent-proto/src/chat.rs#L73): SubmitMessage { submission, channel, text }; Status and Reconcile take `operation`, not the submission
- [`crates/foks-agent/src/chat.rs:782`](../../../crates/foks-agent/src/chat.rs#L782): SubmitMessage maps to session.submit_chat_message with the MAC-bound submission
- [`crates/foks-client/src/realtime/operations/prepare.rs:231`](../../../crates/foks-client/src/realtime/operations/prepare.rs#L231): operation/message id = random_id(), distinct from the submission id
- [`crates/foks-client-app/src/chat.rs:401`](../../../crates/foks-client-app/src/chat.rs#L401): re-submitting the same submission returns the recorded operation instead of sending again
- [`tools/foks-v019-oracle/mcp_rust_live_test.go:39`](../../../tools/foks-v019-oracle/mcp_rust_live_test.go#L39): Go compat test checks only the upstream KV and team tool names and behavior

**Recommendation**

Add 'foks-rs mcp chat --profile P --account-alias A --team T [--allow-send] [--channel ID ...]' as ToolSet::Chat for named teams. Read tools: chat_channels (Channels, joined with Inbox unread counts); chat_unread (Inbox conversations with unread > 0 and their previews); and chat_history {channel, before?} (History), whose structuredContent carries sequence, sender, send_time, text and untrusted: true. Only with --allow-send: chat_send {channel, text, submission_id?}, where submission_id is 32 lowercase hex and is generated when omitted. It calls SubmitMessage and returns {submission_id, operation_id, status}, mapping Prepared to prepared, Confirmed to committed, Rejected or Cancelled to rejected, and Uncertain to unknown. Also chat_status {operation_id}, which calls Status and, when the state is uncertain, Reconcile. Document that a lost chat_send reply is recovered by repeating chat_send with the same submission_id and identical text, which returns the recorded operation without sending again. Bound output with CHAT_HISTORY_BYTES and MAX_OUTPUT_BYTES. Rate-limit sends in the MCP session. Annotate chat_send with openWorldHint: true. Enforce channel allow-lists and send permission in the agent once client sessions and grants exist. There is no FOKS wire change.

**Already tracked:** ISSUES.md: extended chat (edits, reactions, threads, mentions, attachments) is deferred. This proposal uses only Basic chat operations.

<details><summary>Verifier note</summary>

The core claims hold. The MCP tool sets are only Kv and Team (contract.rs:166). Chat Channels, Inbox, History (CHAT_PAGE_ROWS = 50), SubmitMessage, Status and Reconcile exist (chat.rs:73, agent chat.rs:782). Operation::Chat is not restricted to the desktop (operation_allowed only checks readiness), and the Go compat test checks only the upstream KV and team tool names and behavior (mcp_rust_live_test.go:39-49). One part of the recommendation does not work as written. chat_status {submission_id} cannot call Status and then Reconcile, because those actions take the chat operation id. The operation id is a fresh random message id (prepare.rs random_id), not the 16-byte submission id. The submission maps to its operation only by re-issuing SubmitMessage with the same submission id and identical text, which returns the recorded operation through submitted_operation (client-app chat.rs:401), or through a new lookup. So chat_send has to return the operation id, and chat_status has to take it. Chat submission ids are 32 lowercase hex, unlike the v1- KV handle format, so the tool schemas must differ. Basic chat is limited to named teams with direct same-host membership (ISSUES.md). The SubmitMessage evidence line is 73, not 69.

</details>

### agent-mcp-consent-audit

**Add agent-enforced assistant grants, an audit log, revocation and desktop-lock propagation for MCP sessions**

- Type: security
- Priority: medium
- Effort: L
- Layers: agent, protocol, mcp, desktop-native, desktop-ui, docs
- Verification: adjusted

MCP consent is only the --read-only flag set in the assistant host's configuration. A KV-mode session can read any path in the account and in any accessible team, and can rm -r or overwrite. It has no path scope, no confirmation, no record the user can inspect, and cannot be revoked short of killing processes. The agent cannot tell MCP calls from desktop calls, because a Request carries only version, id and operation and the accept loop checks only the peer uid. The desktop app lock gates only the desktop's own Tauri commands. The agent keeps decrypted previews and catalogs in memory and keeps serving MCP and CLI while the desktop shows locked. Book 21 is right that an MCP secret adds no boundary against same-UID code. The gap is a guardrail against a misdirected or prompt-injected assistant, which reads attacker-influenced content (KV files, and chat once exposed) while holding the user's account access.

**Evidence**

- [`crates/foks-mcp/src/contract.rs:166`](../../../crates/foks-mcp/src/contract.rs#L166): ToolSet::names: read_only is the only policy dimension
- [`crates/foks-agent-proto/src/message.rs:69`](../../../crates/foks-agent-proto/src/message.rs#L69): Request { version, id, operation } has no client identity
- [`crates/foks-agent/src/main.rs:314`](../../../crates/foks-agent/src/main.rs#L314): accept loop calls peer_cred() and compares only uid; pid is not recorded
- [`apps/desktop/src-tauri/src/applock.rs:4`](../../../apps/desktop/src-tauri/src/applock.rs#L4): 'Value-returning commands are blocked when locked' applies only to desktop Tauri commands
- [`crates/foks-agent/src/chat.rs:255`](../../../crates/foks-agent/src/chat.rs#L255): 'concealing plaintext while the desktop is locked is not something this process does today'
- [`book/21-mcp.qmd:99`](../../../book/21-mcp.qmd#L99): 'No authentication at the MCP layer, and why' covers authentication only, not scope or audit

**Recommendation**

Deliver in phases, without a FOKS wire change; this bumps only the agent IPC version. Phase 1: Operation::OpenClientSession { kind: Mcp { tool_set, read_only }, profile, account_alias, label } returns a session_id. The agent records the peer pid (SO_PEERCRED or LOCAL_PEERPID) and the start time. MCP passes session_id with every ReadData, PrepareDataWrite, ExecuteDataWrite and status call, and its scope-equality checks account for the new field. The agent keeps a bounded audit ring per session (time, tool, path, team, bytes, outcome). Add ListClientSessions, ClientSessionAudit and RevokeClientSession; a revoked session gets CapabilityDenied, and MCP exits. Phase 2: SetInteractiveLock { locked } from the desktop host. On lock, clear the chat preview cache and catalog cache and refuse plaintext-returning operations to MCP sessions unless their grant allows it. Phase 3: per-(profile, account) grants (path globs, team allow-list, writes, destructive operations), enforced in data.rs before plaintext leaves the agent. Optionally use MCP elicitation for destructive tools when the client supports it. Correct the tool annotations so host approval prompts work as designed. State in book 21 that these are policy guardrails against a misdirected or prompt-injected assistant, not an authentication boundary against same-UID code.

<details><summary>Verifier note</summary>

The core claims hold. ToolSet::names (contract.rs:166) has read_only as its only policy dimension. Request (message.rs:69) has only version, id and operation. The accept loop (main.rs:314) checks only cred.uid(). operation_allowed (main.rs:1604) does not distinguish clients. applock.rs:4 gates only desktop commands. chat.rs:255 says the agent does not conceal plaintext while the desktop is locked. Nothing in the agent, proto or MCP crates implements audit, client sessions, grants or lock propagation, and ISSUES.md does not track it. The recommendation is feasible without a FOKS wire change; it is an agent IPC version bump only. Adding a session_id to DataScope (deny_unknown_fields) needs that bump, and MCP's field-by-field scope comparison has to account for it. Three corrections. (1) The book 21 heading is at line 99, not 114. (2) 'No confirmation' ignores host-side approval. MCP hosts normally ask the user before tool calls and use destructiveHint and readOnlyHint for that, so improving annotations (agent-mcp-tool-metadata) is part of the existing mitigation, alongside --read-only and versioned preconditions. (3) Because of those mitigations and the six-layer scope, medium priority with a phased delivery fits better than high.

</details>

### agent-observability-logs-diagnostics

**Give the agent its own bounded, timestamped log, counters and a Diagnostics operation**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: agent, protocol, cli, desktop-native, desktop-ui
- Verification: adjusted

Agent logging is 20 eprintln! calls with no timestamp, level or request correlation. Where they end up depends on the launcher. The desktop appends stdout and stderr to <state>/agent.log with no size bound or rotation. CLI ensure_agent sends both to /dev/null, so an agent started for MCP leaves no record of failures. Capacity drops are not logged at all. Oversized responses appear only as a generic 'connection failed' line with no operation name. AgentStatus reports background timers but not the agent version, pid, start time, configured limits, lane occupancy or per-operation error counts. ResponseTiming is aggregated only inside the desktop, so CLI and MCP users have nothing to attach to a bug report.

**Evidence**

- [`crates/foks-agent/src/main.rs:287`](../../../crates/foks-agent/src/main.rs#L287): eprintln!("FOKS agent ready: ..."); 20 unstructured eprintln! lines in total (e.g. 317, 322, 351, 535, 887)
- [`crates/foks-agent/src/main.rs:329`](../../../crates/foks-agent/src/main.rs#L329): connection-capacity drop: try_acquire_owned failure drops the stream with no log or counter
- [`crates/foks-agent/src/main.rs:351`](../../../crates/foks-agent/src/main.rs#L351): oversized responses surface only as a generic 'foks-agent connection failed: agent message exceeds its size limit' via write_response (6138)
- [`apps/desktop/src-tauri/src/agent/process.rs:298`](../../../apps/desktop/src-tauri/src/agent/process.rs#L298): agent.log opened append-only (O_NOFOLLOW, 0600); no rotation or size cap; early_exit_error reads from log_offset (279)
- [`crates/foks-cli/src/mcp.rs:62`](../../../crates/foks-cli/src/mcp.rs#L62): .stdout(Stdio::null()).stderr(Stdio::null()) for a CLI-launched agent
- [`crates/foks-agent-proto/src/message.rs:440`](../../../crates/foks-agent-proto/src/message.rs#L440): AgentStatus::Ready carries only timers and history_after
- [`crates/foks-agent/src/main.rs:1796`](../../../crates/foks-agent/src/main.rs#L1796): dispatch_timed builds per-request ResponseTiming that the agent does not aggregate
- [`crates/foks-agent/src/retention.rs:14`](../../../crates/foks-agent/src/retention.rs#L14): existing AtomicU64 retention counters a Diagnostics operation can reuse

**Recommendation**

1) The agent opens its own log: --log-file, defaulting to <state-dir>/agent.log, opened O_NOFOLLOW with mode 0600 and the owner check the desktop performs today. It rotates at 1 MiB with 3 files and writes lines as 'RFC3339 level event key=value', never secrets or KV paths. Launchers keep capturing stderr, or the agent opens the log before argument and lock validation, so the desktop's early-exit diagnostics still see startup failures. 2) Counters: rejected peers, capacity drops, version mismatches, response-too-large with the operation name, and per operation name the count, an error-code histogram and p50/p95 queue and body times, fed from the ResponseTiming dispatch_timed already builds. 3) Operation::Diagnostics: a read with Scope::None, allowed in bootstrap. It returns version, protocol, pid, uptime, effective limits, available permits per lane, active chat polls, timers, the existing retention counters, the new counters and the last 50 warning and error lines. 4) Add 'foks-rs agent diagnostics [--json]', and include the result in the desktop's existing Copy diagnostics.

<details><summary>Verifier note</summary>

Mostly confirmed. The agent has exactly 20 eprintln! calls with no timestamp, level or request id, and no tracing or log crate. The desktop opens <state>/agent.log append-only with no rotation or size cap (process.rs:298-304; no set_len or rotation anywhere). ensure_agent nulls stdout and stderr (mcp.rs:62-64). The accept loop drops connections at capacity silently (main.rs:329-332). AgentStatus::Ready carries only timers and history_after (message.rs:440-453). The CLI has no agent status or diagnostics command. dispatch_timed builds a ResponseTiming per request that the agent never aggregates. The desktop's diagnostics.rs aggregates timings in memory for Copy diagnostics. ISSUES.md does not track this. One claim is wrong: oversized responses are logged, though only generically. encode() returns TooLarge, write_response propagates it, and handle_connection's caller prints 'foks-agent connection failed: agent message exceeds its size limit' with no operation name (main.rs:6138, 1317, 351). Rejected peers are also logged (317, 322); only capacity drops are silent. The agent already keeps retention counters (retention.rs:14-26) exposed through RetentionStatus, which the Diagnostics operation should reuse. The recommendation also needs one refinement. The desktop's early-exit diagnostics read agent.log from the launch offset (process.rs:279), so startup failures that happen before the agent opens its own log, such as argument and lock errors, must still reach a captured stderr.

</details>

### agent-request-deadline

**Carry the client's deadline in each request instead of an agent-wide timeout set by whichever front end launched the agent**

- Type: correctness-risk
- Priority: medium
- Effort: S
- Layers: protocol, agent, client-lib, cli, desktop-native
- Verification: adjusted

The agent applies one --request-timeout-seconds value (default 15) to every request. The desktop launches it with 60 s because macOS Keychain prompts can block longer, and CLI ensure_agent launches it with the default. The desktop uses any compatible agent already on its socket and replaces one only on a version mismatch. So when the CLI is pointed at the desktop's state root and starts the agent first, the desktop's Keychain-bound requests are cut at 15 s, not 60 s. Conversely, a 60 s agent outlives the CLI's 15 s client deadline and the CLI reports an ambiguous outcome. Request has no deadline field, so the agent estimates the client's remaining budget with a fixed 2 s margin. The pairing and chat-poll timeouts are duplicated in the agent and the client.

**Evidence**

- [`crates/foks-agent/src/main.rs:165`](../../../crates/foks-agent/src/main.rs#L165): #[arg(long, default_value_t = 15)] request_timeout_seconds (field at 166); used as the agent-wide timeout at 270
- [`apps/desktop/src-tauri/src/agent/process.rs:221`](../../../apps/desktop/src-tauri/src/agent/process.rs#L221): managed_agent_arguments passes --request-timeout-seconds 60, citing blocking Keychain prompts
- [`apps/desktop/src-tauri/src/agent/process.rs:1024`](../../../apps/desktop/src-tauri/src/agent/process.rs#L1024): start_already_reserved uses any agent that answers probe_status; it replaces one only on version-mismatch (1062)
- [`crates/foks-cli/src/mcp.rs:51`](../../../crates/foks-cli/src/mcp.rs#L51): ensure_agent launches the sibling foks-agent with only --state-dir
- [`apps/desktop/src/app/blocking-shell.tsx:28`](../../../apps/desktop/src/app/blocking-shell.tsx#L28): desktop tells users to run foks-rs --state-dir <desktop root>, so the CLI and the desktop can share one agent
- [`crates/foks-agent/src/main.rs:68`](../../../crates/foks-agent/src/main.rs#L68): ADMISSION_REPLY_MARGIN fixed at 2 s; admission_budget at 698; applied at 1216/1233
- [`crates/foks-agent-client/src/lib.rs:20`](../../../crates/foks-agent-client/src/lib.rs#L20): DEVICE_PAIRING_TIMEOUT and CHAT_POLL_TIMEOUT duplicated from agent main.rs:73-74
- [`apps/desktop/src/scheduling/profile-work.ts:63`](../../../apps/desktop/src/scheduling/profile-work.ts#L63): NATIVE_REQUEST_TIMEOUT = 60_000 assumes the desktop's --request-timeout-seconds 60

**Recommendation**

Add #[serde(default, skip_serializing_if = "Option::is_none")] budget_ms: Option<u32> to Request, holding the client's remaining budget at send time; AgentClient fills it from its own timeout. The agent uses min(budget_ms, --maximum-request-timeout-seconds), with a default maximum of 300 s matching AgentClient::set_timeout's bound. It still subtracts a reply margin from that budget, because budget_ms excludes connect time but not request read, scheduling or reply transit; the margin can be smaller than 2 s but not zero. Keep --request-timeout-seconds as the default for requests without budget_ms, for scheduler jobs and for write deadlines. Move DEVICE_PAIRING_TIMEOUT and CHAT_POLL_TIMEOUT into foks-agent-proto. Add foks_agent_client::launch::arguments(state_dir, socket), used by both managed_agent_arguments and ensure_agent. Stop passing --request-timeout-seconds from the desktop only after budget_ms ships, and derive profile-work.ts's NATIVE_REQUEST_TIMEOUT from the client timeout rather than the agent flag.

<details><summary>Verifier note</summary>

The core claim holds. The agent takes one --request-timeout-seconds value with a default of 15 (main.rs:165-166, not 161). The desktop passes 60 (process.rs:221-231), and the desktop client's own timeout is 60 s (agent.rs:309). CLI ensure_agent passes only --state-dir (mcp.rs:51-66). Request has only version, id and operation (message.rs:69-73). admission_budget subtracts a fixed ADMISSION_REPLY_MARGIN (main.rs:68, 698). DEVICE_PAIRING_TIMEOUT and CHAT_POLL_TIMEOUT are duplicated (main.rs:73-74 and agent-client lib.rs:20-21). The desktop uses any compatible agent that answers probe_status (process.rs:1024-1028, 1056-1059) and replaces one only on version-mismatch. The scenario therefore holds when the CLI is pointed at the desktop's state root, which blocking-shell.tsx:28-29 tells users to do. The reverse case also exists: a 60 s desktop-launched agent keeps running a CLI request after the CLI's 15 s client deadline, so the CLI reports an ambiguous outcome. ISSUES.md does not track this. The recommendation has one flaw. budget_ms measured at send time excludes connect time but not request read, scheduling or reply transit. An agent that waits the full budget_ms in admission would still answer after the client gives up, which is the failure the margin comment describes, so a reply margin must stay when budget_ms is present. The agent-wide timeout is also still used for scheduler jobs and write_response, and profile-work.ts:58-70 derives NATIVE_REQUEST_TIMEOUT from the desktop's 60 s flag.

</details>

### agent-mcp-tool-metadata

**Write real MCP tool descriptions and correct the tool annotations**

- Type: feature-refinement
- Priority: low
- Effort: S
- Layers: mcp
- Verification: adjusted

Every tool description is generated as 'FOKS KV {name} in the selected account' (or 'FOKS team {name} ...'), and no parameter has a description. Annotations set idempotentHint false everywhere and destructiveHint true on every write, including mkdir, which only adds. The rules that make MCP writes exactly-once (reuse the submission_id, never issue a new id after status unknown, call foks_status) are not in the tool metadata or server instructions; they appear only in some error result text. foks_status and foks_pending are described as 'FOKS KV foks_status in the selected account'.

**Evidence**

- [`crates/foks-mcp/src/contract.rs:226`](../../../crates/foks-mcp/src/contract.rs#L226): description: format!("FOKS {} {name} in the selected account", KV or team)
- [`crates/foks-mcp/src/contract.rs:228`](../../../crates/foks-mcp/src/contract.rs#L228): annotations: readOnlyHint = !writes (already true for reads), destructiveHint = writes, idempotentHint false for all
- [`crates/foks-mcp/src/session.rs:44`](../../../crates/foks-mcp/src/session.rs#L44): get_info sets no server instructions
- [`crates/foks-mcp/src/agent/writes.rs:161`](../../../crates/foks-mcp/src/agent/writes.rs#L161): unknown-outcome guidance appears only in error result text, not in tool metadata

**Recommendation**

Replace the generated strings with a static table of descriptions for each tool and parameter. Cover path semantics, the team selector, base64, mkdir_p, overwrite, recursive, the submission_id format and reuse rule, and the 4 MiB limit. Keep readOnlyHint: true on reads; idempotentHint is ignored there. Annotations cannot vary with arguments, so set mkdir to destructiveHint: false and keep put, rm and mv at destructiveHint: true, because put can overwrite. Describe the overwrite flag's effect in put's description. Set ServerInfo.instructions to describe the unknown-outcome rule (reuse the submission_id, never issue a new id after status unknown) and when to call foks_status and foks_pending. Add a snapshot test of the tools/list output so description and annotation changes are reviewed.

<details><summary>Verifier note</summary>

The core claims hold. contract.rs:226 generates 'FOKS {KV|team} {name} in the selected account'. No parameter has a description. contract.rs:228 sets idempotentHint false for every tool and destructiveHint true for put, mkdir, rm and mv. get_info (session.rs:44) sets no instructions. The submission_id rules do not appear in tool metadata, although writes.rs:161 puts some guidance in error result text. The Go compat test checks only names and behavior, so changing descriptions does not affect compatibility. Three corrections. (1) The summary says every description reads 'FOKS KV ...'; team tools read 'FOKS team ...'. (2) Reads already have readOnlyHint: true (!writes). Under the MCP ToolAnnotations definition, idempotentHint and destructiveHint mean something only when readOnlyHint is false, so setting idempotentHint: true on reads changes nothing. (3) Annotations are static per tool, so put cannot be destructive 'only when overwrite is set'. put can overwrite, so destructiveHint: true is correct for it. Only mkdir should change to destructiveHint: false.

</details>

### agent-operation-policy-table

**Snapshot every operation's policy row so the eight classifiers cannot drift apart**

- Type: testing
- Priority: low
- Effort: S
- Layers: agent, protocol
- Verification: adjusted

Each operation is classified separately by is_mutation, operation_scope, operation_shares_profile, operation_serves_reads, operation_leaves_retained_material, operation_is_abandonable, operation_is_noninteractive and ConnectionCapacity::worker_pool. policy.rs explains why these stay separate. Only operation_scope is an exhaustive match; the others are allow-lists with a conservative default, and no test enumerates every operation. They already disagree. BindDataAccount and PassphraseStatus are local reads, IPC non-mutations, and BindDataAccount is noninteractive. Neither appears in either cache list, so each call flushes every read cache and the catalog cache before and after dispatch. That happens on each MCP backend connect (plus 25 ms retries while ProfileBusy) and on each desktop passphrase-status read. Because the defaults are conservative, drift costs performance, not safety.

**Evidence**

- [`crates/foks-agent/src/profile_work/policy.rs:1`](../../../crates/foks-agent/src/profile_work/policy.rs#L1): module doc lists the policy dimensions and their owners; operation_scope is the only exhaustive match
- [`crates/foks-agent/src/read_cache.rs:444`](../../../crates/foks-agent/src/read_cache.rs#L444): operation_serves_reads / leaves_retained_material (493) / is_abandonable (552) / shares_profile (581) are allow-lists
- [`crates/foks-agent-proto/src/message.rs:1245`](../../../crates/foks-agent-proto/src/message.rs#L1245): is_mutation allow-list includes BindDataAccount and PassphraseStatus
- [`crates/foks-agent/src/main.rs:3232`](../../../crates/foks-agent/src/main.rs#L3232): operation_is_noninteractive includes BindDataAccount, BotAccount List and Sso Status
- [`crates/foks-agent/src/main.rs:3310`](../../../crates/foks-agent/src/main.rs#L3310): invalidates = !reads && !leaves_retained_material && scope is not None/RegistryRead; flushes all read and catalog caches
- [`crates/foks-mcp/src/agent.rs:35`](../../../crates/foks-mcp/src/agent.rs#L35): BindDataAccount is sent once per MCP backend connect; bind_account (485) retries every 25 ms only on ProfileBusy, for up to 5 s
- [`apps/desktop/src-tauri/src/commands/accounts.rs:814`](../../../apps/desktop/src-tauri/src/commands/accounts.rs#L814): desktop issues PassphraseStatus, which flushes every cache

**Recommendation**

Keep the dimensions separate, as policy.rs intends. Add a test helper all_operation_samples() that returns one sample per Operation variant and per ChatAction, InvitationAction, BotAction and SsoAction arm. An exhaustive match on the variant makes a new variant fail to compile until it has a sample. Then add a snapshot test that renders 'name | is_mutation | scope | shares | serves_reads | leaves_material | abandonable | noninteractive | pool' for every sample into a checked-in file (e.g. src/profile_work/policy/operations.snap), so a new operation's row appears in review. Review BindDataAccount, PassphraseStatus, Sso Status and BotAccount List, and add them to operation_leaves_retained_material if they write no credential state.

<details><summary>Verifier note</summary>

The core claim holds. policy.rs:1-19 documents the separate dimensions. Only operation_scope is an exhaustive match. operation_serves_reads (read_cache.rs:444), operation_leaves_retained_material (493), operation_shares_profile (581), operation_is_noninteractive (main.rs:3232) and worker_pool (main.rs:131) all end in `_ =>` defaults, and is_mutation is a `!matches!` allow-list (message.rs:1245). Existing tests use hand-picked `for operation in [...]` lists, none exhaustive. BindDataAccount and PassphraseStatus are IPC non-mutations, and BindDataAccount is noninteractive. Both have profile scope (policy.rs:102, 122) and appear in neither cache list, so dispatch_result (main.rs:3310-3320) calls read_cache::invalidate_all and invalidate_cached_catalogs before and after each call. Both only read: data::bind_account calls session.data_identity, and PassphraseStatus calls passphrase_status. Two corrections lower the priority. First, every allow-list defaults to the conservative side (invalidating, exclusive, interactive, mutation), so drift costs cache hits and concurrency rather than safety. Second, the MCP backend calls bind_account once per connect (foks-mcp agent.rs:35); it retries every 25 ms only while ProfileBusy comes back, for up to 5 s. The desktop's account_passphrase_status is the other caller. The effect is a global cache flush per call, a performance cost. The snapshot-test recommendation is sound and cheap.

</details>
