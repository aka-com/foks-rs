# Outstanding issues

This inventory records gaps found while reviewing the team-chat and team-invitation
plans against the current implementation, and one desktop maintainability gap.
It separates current implementation risks from limitations that the plans
already disclose and from documentation-only maintenance. Extended chat remains deferred; its prerequisites are recorded here
without making them current implementation work.

## Stage 1 — Basic chat server retention and storage isolation

**Priority: high.** Basic chat has no chat-specific retained-byte, message-count,
account, team or channel storage quota. Each `rt_messages` row stores both the
submitted envelope and the exact returned message, and each field permits up to
1 MiB. There is no message deletion or compaction path. The server's global SQLite
size ceiling prevents unlimited filesystem growth, but one authorized member can
consume that shared capacity and interfere with unrelated host services.

Evidence:

- [`crates/foks-server-db/src/schema/realtime.sql`](crates/foks-server-db/src/schema/realtime.sql)
  defines permanent `rt_messages` rows and the two large encodings.
- [`crates/foks-server-db/src/realtime/limits.rs`](crates/foks-server-db/src/realtime/limits.rs)
  bounds individual requests and responses but defines no retained chat-storage
  budget.
- No production path deletes from `rt_messages`.

Required resolution:

- Define per-account/team/channel retained byte and message limits, plus an
  operator-wide chat budget that cannot consume the database reserve needed by
  account, team, invitation and KV writes.
- Enforce admission and existing-idempotency-record recovery in the same writer transaction.
- Choose explicit retention, archival and deletion behavior before presenting a
  quota as a complete lifecycle.
- Add usage metrics, capacity diagnostics and tests showing one team cannot starve
  unrelated writes.

## Stage 2 — Basic chat local terminal-operation retention

**Priority: medium.** Every prepared Basic chat operation can leave a permanent
`chat_operations` row and a permanent `chat_submissions` row after confirmation,
rejection or cancellation. Protected request material is removed after a terminal
transition, but the hard-state identity rows have no timestamp, tombstone or
compaction lifecycle. The 1,000-operation admission limit counts only prepared and
uncertain rows, so it does not bound total local growth.

Evidence:

- [`crates/foks-client-db/src/schema.rs`](crates/foks-client-db/src/schema.rs)
  defines the two tables without lifecycle timestamps.
- [`crates/foks-client-db/src/repositories/chat.rs`](crates/foks-client-db/src/repositories/chat.rs)
  enforces pending capacity and terminal transitions but has no terminal-row
  compaction.

Required resolution:

- Preserve unresolved operations indefinitely.
- Compact terminal operations into bounded replay evidence that retains submission
  binding, scope, request commitment and outcome.
- Reject reuse safely after compact evidence expires; deletion must not make an old
  submission identifier appear new.
- Coordinate this with the protected-store reconciliation machinery, while keeping
  chat ledger policy separate from protected-object collection.

## Stage 3 — Invitation server retention and permanent capacity exhaustion

**Priority: medium.** Invitation certificates and local/remote request rows retain
terminal history indefinitely. Their hard global limits count historical rows, so
normal long-term use eventually reaches a permanent refusal state: 65,536 remote
requests, 65,536 certificates, and 1,000,000 local requests. Terminal request
history also retains plaintext local joiner identities and encrypted remote request
payloads without a stated data-retention period.

Evidence:

- [`crates/foks-server-db/src/schema/team_invitations.sql`](crates/foks-server-db/src/schema/team_invitations.sql)
  records creation and decision metadata but has no tombstone or deletion lifecycle.
- [`crates/foks-server-db/src/team_invitations.rs`](crates/foks-server-db/src/team_invitations.rs)
  counts all certificate and request rows for global admission caps. Certificates
  are not age-pruned, and recovery from the global terminal-row limits is
  undefined.

Required resolution:

- Specify retention separately for active requests, terminal decision evidence,
  certificates still referenced by requests, and obsolete unreferenced
  certificates.
- Keep enough compact decision evidence to prevent unsafe replay and to explain
  ambiguous outcomes.
- Reserve capacity for review, rejection, approval and cleanup even when new
  submissions are refused.
- Prove foreign-key-safe batch collection, restart behavior, concurrent decisions
  and operator observability.

## Stage 4 — Invitation revocation, withdrawal and permission lifecycle

**Priority: medium.** A shareable invitation has no expiry or administrator revoke
operation. Older certificates remain valid after admin-key rotation. An applicant
can cancel only a request that has not been submitted; there is no requester
withdrawal operation for a pending request.

Local acceptance also grants the destination team permission to view the applicant
before inserting the pending request. Rejecting the request changes only the request
state; it does not revoke `team_local_view_permissions`. Remote view permissions
have lower-level expiry and revocation support, but invitation rejection and
withdrawal do not drive it. Rejected or abandoned invitation workflows can therefore
leave broader identity-view access than their final state suggests.

Required resolution:

- Decide whether revocation and withdrawal are Rust-only policy extensions or need
  a new interoperable protocol operation.
- Add administrator disable/rotate behavior for leaked links without treating key
  rotation as implicit revocation.
- Define whether rejection, withdrawal and expiry revoke invitation-created local
  and remote view permissions. Preserve permissions required by committed
  membership.
- Expose the resulting state honestly when a Go host cannot supply equivalent
  status or revocation evidence.

## Stage 5 — Invitation abuse resistance

**Priority: medium.** Every successful remote submission creates a fresh RSVP,
including identical ciphertext. A holder of a copied invitation can repeatedly
fill a team's 1,000 pending slots, force administrative review and consume the
permanent 65,536-row remote-request budget. General connection/IP rate limiting
reduces single-source floods but does not provide per-certificate accounting or a
way to disable the abused link.

Required resolution:

- Add per-certificate and per-team submission budgets that preserve ordinary Go
  interoperability.
- Combine admission limits with certificate disable and terminal-row retention so
  an attacker cannot turn cleanup into immediate re-admission forever.
- Add distributed-source flood tests and prove that authenticated administrators
  can still list and decide existing requests at capacity.

## Deferred extended-chat prerequisite

The extension capability method uses a locally selected method position rather
than a reserved upstream or vendor namespace. Collision checks cover the pinned
Go protocol, but a future upstream release could allocate the same position.
Before any extended-chat capability is advertised, assign a durable protocol or
method namespace and test negotiation against newer upstream registries.

The remaining extended-chat work stays deferred. In particular, reaction
equality, exact event-body wire forms, method/status allocation, attachment
authorization and committed-object retention are designs rather than enabled
features.

## Desktop operation builders still built twice

**Priority: low.** `foks_desktop::ops` holds the agent `Operation` builders and
field checks shared by the Tauri command layer and `foks-desktop-backend`. The
backend therefore applies the shipping input rules for account creation, owner
backup, owner recovery, team creation and membership, and KV mutations, and the
real-agent transcript (`scripts/test-foks-desktop-real-agent.sh`) exercises them.
The server profile, passphrase, security key and device pairing operations are
still built separately in both callers, so their rules can drift. Some inputs
differ today: the backend's `passphrase-change` sends no current passphrase,
while the Tauri command can. The server status response validators in
`apps/desktop/src-tauri/src/commands/servers.rs` exist only in the Tauri layer.
These were left out of the first changes to keep each one reviewable, and
each needs its own decision on which caller's rule to keep.

Required resolution:

- Move the remaining builders and response validators into `foks_desktop::ops`
  modules and have both callers use them, as `ops::accounts` and `ops::groups`
  do.
- The promotion and demotion ordering checks stay in the Tauri layer, because
  only it holds the member's current role from the authenticated roster.

## Chunk temporaries left by earlier releases stay charged

**Priority: low.** Before the filesystem chunk store named its temporaries
`_foks_tmp-*`, it used the tempfile default (`.tmp` plus six random
characters). A process killed between creating and persisting one left a file
that inventory cannot tell from a foreign object. Inventory records it as an
unknown object, which stays charged against `maximum_chunk_physical_bytes` and
`maximum_chunk_objects`, and collection never deletes unknown objects.

Required resolution:

- Decide whether an operator command may remove `.tmp??????` files from a
  filesystem namespace, with the same writer guard, exclusive namespace lease
  and authority checks as collection, and release their ledger rows; or keep
  them charged and document how to remove them by hand.

## Existing disclosed limitations

These are real product or verification limits, but the reviewed plans already
identify them accurately:

- Basic chat supports named teams with direct same-host user membership. Ad-hoc,
  nested and federated team chat are unsupported.
- The pinned Go filtered-inbox behavior can prevent exact unread completeness;
  FOKS-RS reports degraded state instead of inventing progress.
- Cross-restart equivocation checking retains only the latest 10,000 message
  anchors per account/channel and does not provide a globally complete or
  Merkle-authenticated chat log.
- Local invitation approval can race rejection because the Go-compatible team edit
  has no RSVP precondition.
- Lost local acceptance, remote acceptance and rejection replies can remain
  permanently unknown when no authorized exact read-back exists.
- The pinned Go client cannot open rotated invitation certificates because its
  stacked-signature verifier uses the reverse order.
- Reliable minimized/background operation, native multi-window ownership and
  post-quit operation remain deferred.
- Extended channel management, edits, deletion, reactions, threads, mentions and
  attachments remain disabled. The desktop's Mentions only alert mode matches
  `@username` in message text on the device; it is not a protocol mention.
- The pinned protocol has no request that sets a conversation's `muted` or
  `hidden` flag, and the server reports both as false. The desktop keeps
  channel and team mutes and alert modes in device-local settings, which do not
  sync between devices; a desktop build from before those settings cannot read
  a settings file that holds a mute, a team default or Mentions only.
- The larger real-desktop catch-up and before/after foreground-latency benchmark
  has not been run.
