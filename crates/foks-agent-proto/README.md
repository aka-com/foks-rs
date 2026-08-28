# foks-agent-proto

Versioned local IPC types shared by the standalone FOKS agent and its clients.
Messages are JSON inside a four-byte big-endian length frame, capped at 1 MiB.
Every response is bound to a request ID and contains either a JSON value or a
stable error category.

The v1 operation set is intentionally non-secret-bearing: status, profile and
account listing, probe, user/team synchronization, KV listing, and due-job
execution. Signup, provisioning, recovery phrases, and mutations remain on the
direct CLI/application boundary.
