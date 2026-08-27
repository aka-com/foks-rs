# foks-server-db

The authoritative SQLite storage layer for the standalone FOKS server. One
writer owns a dedicated database opened in WAL mode; snapshots use separately
configured read connections. All signed and encrypted protocol objects are
stored as exact blobs. This crate contains no network or AKA dependencies.

The schema is pre-release v1. Incompatible development changes replace v1
rather than adding migration compatibility.
