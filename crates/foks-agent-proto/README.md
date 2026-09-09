# foks-agent-proto

Versioned local IPC types shared by the standalone FOKS agent and its clients.
Messages are JSON inside a four-byte big-endian length frame, capped at 1 MiB.
Every response is bound to a request ID and contains either a JSON value or a
stable error category.

The v2 operation set includes bootstrap/ready status, state initialization,
profile lifecycle, profile/account listing, probe,
user/team synchronization, paged KV listing, version-bound KV reads and chunks,
compare-and-swap account/team-store mutations, same-socket streaming file uploads,
due-job execution, software signup,
and explicitly two-profile remote-team admission/listing.
Native named/ad-hoc team creation and its durable resume are separate mutation
operations; no rename or close operation is implied.
Signup can carry an invite and optional passphrase over the authenticated,
private local socket; passphrase set/change/verify are also explicit operations.
Secret fields are redacted from diagnostics, and encoded/decoded agent request
buffers are zeroized. The agent generates and stores device secrets itself.
Owner-device provision/resume, device listing, backup enrollment, account
recovery/resume, and YubiKey passphrase operations use the same v2 boundary.
Device summaries carry an optional display name only when the authenticated
user-chain response disclosed the commitment opening. Team demotion and removal
select the exact authenticated local-user roster row by party ID; username
is used only when adding a new member.
Recovery phrases and hardware PINs use redacted, zeroizing request fields; the
newly generated backup phrase is the sole secret response and is returned once
so the frontend can place it in offline storage.

Before credentials exist, the resident agent admits only `AgentStatus` and
`InitializeState`; normal requests receive `BootstrapRequired`. Profile-add requests carry
the native protocol policy and trust-root choice explicitly. They do not infer a server,
account, or local content source from desktop state.

Federation operations name both profiles and both protected team aliases and
carry only a role/visibility selection. They never carry bearer permissions,
PTKs, removal keys, or checkpoint material. Responses remain ordinary JSON
values rather than a second DTO hierarchy.

Store entry creation requires a
`Create` precondition, and updates or removals require `ExactVersion`; the agent
does not infer overwrite intent. Inline values and stream chunks are limited to 128
KiB so their JSON representation stays below the 1 MiB frame ceiling. A stream
holds mutation single-flight from its header through commit, rejects gaps and
rebinding, and treats a missing post-commit response as an ambiguous outcome.
