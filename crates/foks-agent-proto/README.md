# foks-agent-proto

Versioned local IPC types shared by the standalone FOKS agent and its clients.
Messages are JSON inside a four-byte big-endian length frame, capped at 1 MiB.
Every response is bound to a request ID and contains either a JSON value or a
stable error category.

The v1 operation set includes status, profile/account listing, probe,
user/team synchronization, KV listing, due-job execution, software signup,
and explicitly two-profile remote-team admission/listing.
Signup can carry an invite and optional passphrase over the authenticated,
private local socket; passphrase set/change/verify are also explicit operations.
Secret fields are redacted from diagnostics, and encoded/decoded agent request
buffers are zeroized. The agent generates and stores device secrets itself.
Provisioning and recovery phrases remain on the direct CLI/application
boundary.

Federation operations name both profiles and both protected team aliases and
carry only a role/visibility selection. They never carry bearer permissions,
PTKs, removal keys, or checkpoint material. Responses remain ordinary JSON
values rather than a second DTO hierarchy.
