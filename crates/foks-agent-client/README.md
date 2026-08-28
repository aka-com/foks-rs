# foks-agent-client

Reusable synchronous client for `foks-agent-proto`. On Unix it verifies that
the target is a private Unix-domain socket, applies bounded I/O timeouts,
enforces the protocol frame limit, and checks response/request ID binding.

Windows named-pipe transport and peer-identity validation are not implemented;
the crate returns an unsupported-platform error there.
