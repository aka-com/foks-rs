# foks-server

Standalone FOKS v0.1.9 server composition. The current implementation provides
the strict RPC/router boundary, three policy-separated TLS listeners, bounded
connection workers, detached-device client-certificate issuance, graceful
shutdown, and integration with the authoritative SQLite crate.

At this implementation checkpoint the probe body and TLS identities are
supplied as already-validated configuration. Hostchain/public-zone bootstrap,
encrypted production key persistence, database-bound client-certificate
authorization, and registration/KV handlers are subsequent phases; the shell
returns the stable v0.1.9 unsupported status for those handlers until they are
installed. `foks-server-testkit` supplies only temporary loopback composition
and is not a production dependency.
