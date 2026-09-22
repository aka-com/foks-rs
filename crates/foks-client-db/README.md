# foks-client-db

`foks-client-db` is the durable hard-state boundary for a native FOKS
client. It deliberately contains no application-layer vault types.

The store accepts a host snapshot only after a protocol verifier has checked
the exact Snowpack objects, signatures, host-chain extension, service
delegations, and Merkle signature. In one SQLite transaction it persists:

- the discovery name to permanent HostID pin;
- the exact authenticated host-chain bytes and indexed chain tail;
- the currently delegated service endpoints; and
- the exact latest signed Merkle-root bytes and indexed epoch/hash;
- verified user-chain bytes and their Merkle root; and
- public device and PUK identifiers/HEPKs.

The database applies additional monotonicity checks. It rejects lookup-name
identity substitution, chain or Merkle rollback, same-sequence forks, and
changes to a host projection that are not accompanied by a chain advance.
These checks are defense in depth; they are not a replacement for FOKS chain
or Merkle verification.

Merkle heads are stored as flat trusted-local checkpoints. Schema 38 upgrades
to 39 atomically using an iterative legacy reader and signed-head verification;
all historical roots survive migration. Reference-aware collection retains
heads, external checkpoint pins, and roots needed by every stored user/team
proof segment, signed link, generic chain and import completion marker. Callers must hold
exclusive profile access and account for private resumable material; unfinished
SQLite workflows defer collection. See [the design and reviews](../../docs/merkle-checkpoints.md).

Private device and PUK seeds are expressly excluded and belong in an encrypted
key store, not ordinary hard-state rows.

Hard state is organized as host, user, team, mutation/chat journal, SSO flow,
job, and metadata repositories over one connection-owning `HardStateStore`.
Every repository uses the same immediate-write transaction layer; the split
does not create independent connections or weaken atomic
host/Merkle/user/team acceptance.

The separate soft-state schema stores complete verified KV roots,
directories, dirent versions, and content projections. It reconstructs durable
FOKS path-version vectors for cache checks, atomically replaces and prunes
stale directory subtrees, and stages large-file plaintext in bounded chunk
rows. Incomplete stages are invisible and normally reclaimed by the store that
created them; an explicit exclusive-maintenance call reclaims stages left by a
process crash. Completed files are read through a streaming writer API.
