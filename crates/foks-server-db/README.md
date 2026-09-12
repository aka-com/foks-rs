# foks-server-db

The authoritative SQLite storage layer for the standalone FOKS server. One
writer owns a dedicated database opened in WAL mode; snapshots use separately
configured read connections. All signed and encrypted protocol objects are
stored as exact blobs. This crate contains no network or AKA dependencies.
Signup invites and passphrase salts, encrypted PPE generations, one-time login
challenges, and bounded failed-proof accounting use the same single-writer
transaction boundary as identity state.

That writer also owns username mutations, invitation certificates and pending
requests, scoped team view grants and removals, realtime channels/messages/inbox
versions, and encrypted OIDC sessions and access state. Authorization and
capacity are rechecked in the committing transaction; read snapshots never
publish a partially committed mutation or inbox wakeup.

Database opens reject a symlink at the database leaf. On Unix, the main
database must be a regular file with exactly one hardlink and a stable nonzero
device/inode identity; unsupported filesystem identity semantics fail closed.
The server writer retains its adjacent path lock and rechecks the captured
identity and link count under that lock before opening SQLite.

The schema is pre-release v1. Incompatible development changes replace v1
rather than adding migration compatibility.
