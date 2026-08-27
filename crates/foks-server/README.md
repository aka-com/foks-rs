# foks-server

Standalone, single-host FOKS v0.1.9 server backed by a dedicated SQLite
database. The implemented shell creates encrypted named host keys, constructs
and verifies a fresh genesis hostchain/public zone/Merkle root, binds three
policy-separated TLS listeners, and serves Probe plus current and historical
Merkle roots from validated read-only SQLite snapshots. One bounded writer
actor owns authoritative SQLite mutations; network work runs in fixed worker
pools with bounded queues and shutdown cancellation.

Username reservation and software-device signup are implemented. Signup
strictly decodes the narrow v0.1.9 request, verifies both eldest signatures and
all public disclosures/bindings, consumes the exact 17-byte expiring
reservation, commits identity state and Merkle epoch 2 atomically, signs the
new root, and records a replay receipt. Public client-certificate lookup issues
and persists a seven-day certificate for the exact enrolled Ed25519 device key;
the certificate completes mTLS against the installation's private client CA.
Application-level active-device binding, user-chain reads, and KV handlers are
not implemented yet; they return the stable v0.1.9 unsupported status.
Consequently the existing client's full account-creation API commits signup,
obtains a working certificate, and then stops at user-chain loading.
Registration rate limiting is not implemented yet. Merkle history is currently
limited to 64 full roots and 64 hashes per call and opens one read-only SQLite
connection per request. The authenticated listener already requires a
certificate from the installation's private client CA, but active device
authorization is intentionally not claimed until those handlers land.

## Running the shell

The probe certificate must be publicly trusted by clients and cover the
canonical hostname. Service listener certificates are generated under the
delegated CA authenticated by the hostchain. All state paths and the 32-byte
operator root-key file are explicit; there is no user-home or AKA data-path
default.

```text
foks-server \
  --canonical-name foks.example.test \
  --database /srv/foks/foks-server.sqlite \
  --key-directory /srv/foks/keys \
  --root-key-file /run/secrets/foks-root-key \
  --probe-certificate-der /run/secrets/probe-leaf.der \
  --probe-certificate-der /run/secrets/probe-ca.der \
  --probe-private-key-der /run/secrets/probe-key.pk8 \
  --probe-address 0.0.0.0:4430 \
  --public-address 0.0.0.0:4431 \
  --authenticated-address 0.0.0.0:4432
```

The root-key and probe-private-key files must be regular, non-symlink files
with no group or other permissions. `SIGINT` and `SIGTERM` stop accepts,
cancel idle sessions, drain accepted database work, and join the writer.

`foks-server-testkit` composes the same path only inside owned temporary
directories and ephemeral loopback sockets. It is publish-disabled and is not
a production dependency.
