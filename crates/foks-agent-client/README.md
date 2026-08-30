# foks-agent-client

Reusable synchronous client for `foks-agent-proto`. On Unix it verifies that
the target is a private Unix-domain socket, applies bounded I/O timeouts,
enforces the protocol frame limit, and checks response/request ID binding.

Windows named-pipe transport and peer-identity validation are not implemented;
the crate returns an unsupported-platform error there.

Unix peer authentication is deliberately scoped to the OS user ID. The socket
path and mode checks prevent cross-user access and accidental redirection, but
they do not distinguish mutually untrusted processes owned by the same UID.
