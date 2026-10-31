# foks-agent-client

Reusable synchronous client for `foks-agent-proto`. On Unix it verifies that
the target is a private Unix-domain socket, applies bounded I/O timeouts,
enforces the protocol frame limit, and checks response/request ID binding.
Large KV writes use one authenticated socket with ordered 128 KiB frames and a
final commit. Failures before that commit are ordinary I/O failures; failures
while sending it or awaiting its response are classified as ambiguous so a UI
can reconcile the exact store path instead of retrying blindly. The agent's
`version-mismatch` refusal is the exception: an agent of another version
refuses a request before decoding it, so that refusal is returned as a protocol
error, never as ambiguous. `AgentClient::hello` exchanges the
version-independent Hello and fails with the same error when the agent does not
accept this build's version.

Windows named-pipe transport and peer-identity validation are not implemented;
the crate returns an unsupported-platform error there.

Unix peer authentication is deliberately scoped to the OS user ID. The socket
path and mode checks prevent cross-user access and accidental redirection, but
they do not distinguish mutually untrusted processes owned by the same UID.
