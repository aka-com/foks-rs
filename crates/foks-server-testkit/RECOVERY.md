# Recovery experiments

This suite measures local process restart and independent-directory restore
experiments, not actual host/AZ recovery. The process-restart baseline retains
the original SQLite volume. Additional experiments delete fixture-owned server
files and recover from a snapshot copied outside that directory. All processes
and files still share one host. There is no replication, distributed fencing,
automatic promotion, routing failover, or power-loss emulation. Passing the
suite does not certify hosted HA; some tests pass by proving missing guarantees.

Build the exact executable being measured before every run:

```sh
cargo build --locked -j 2 -p foks-server --bin foks-server
FOKS_SERVER_BIN="$PWD/target/debug/foks-server" \
  cargo test --locked -j 2 -p foks-server-testkit --test recovery_harness \
  -- --ignored --test-threads=1 --nocapture
cargo test --locked -j 2 -p foks-server-testkit --test retry_recovery \
  signup_commit_with_lost_or_partial_response_reconciles_without_replay
```

The binary tests are explicitly ignored in ordinary runs. Opting in without
`FOKS_SERVER_BIN` fails rather than silently skipping. Use an absolute binary
path; adjust it if using `CARGO_TARGET_DIR`. Run serially to keep timing evidence
useful and avoid competing recovery workloads on a development machine.

`FOKS_RECOVERY_BUDGET_MS` configures the positive recovery budget (default 15000).
The clock starts immediately before killing the server and stops only after
reconnection, authenticated reads of **all** prior acknowledged files, a new
authenticated write, and readback. Logging/fsync time is included. This is a
post-completion latency assertion, not a cancellation deadline; an individual
blocking RPC can exceed the budget before the test reports failure. It measures
three cycles on a small synthetic credential/note workload, not a latency bound
over all sizes or system loads.

For the crash test, set `FOKS_RECOVERY_LEDGER` to a new file path outside the
server volume to retain JSONL evidence after the test. The file must not exist.
Without this variable the fixture removes it with its temporary directory.
The controller fsyncs each acknowledgment and read observation and compares
SHA-256 payload digests. Identifiers are synthetic and payloads are not logged.
Use only synthetic data: digests do not protect low-entropy real passwords.
The original client hard state, soft state, credentials, and protected mutation
store are reused after every crash. Content sync makes fresh authenticated RPCs;
expectations come from the controller ledger, not restored server data.

Separate stale-backup experiments retain client checkpoints and require
specific overwritten-dirent and Merkle rollback errors. A third experiment
demonstrates a real limitation: restoring before a new dirent was inserted can
look like a legitimate deletion and succeed. Directory encryption versions do
not increase on every insertion, and absent dirents are not checked as returned
older versions. The controller ledger still rejects recovery because an
acknowledged insertion disappeared, even when new writes succeed. Do not treat
client rollback checks as a complete data-loss detector. Rejecting all absent
dirents would also reject legitimate unlink operations; resolving this ambiguity
needs separate compatibility and protocol design review. The identity experiment
advances a second account, then has the first client observe that state: a read
alone can make an old backup unacceptable. The retry fixture covers a lost or
partial signup response, restarts both sides, and checks reconciliation without
a second Merkle publication. It uses in-process deterministic fault hooks and a
graceful server restart; it is not an instruction-level process-crash experiment.

## Independent-directory and former-owner experiments

The `volume` module copies a completed backup to a separate controller-owned
directory before injecting failure. The positive case deletes the source server
database, keys, local backups and configuration, then restores into another
directory and validates all expected reads plus a new write/read. Client stores
and controller-held operator-root material survive. A complete quiescent snapshot
is a prerequisite of this positive fixture, not a replication implementation.

A stale-snapshot variant adds an acknowledged insertion after copying the backup.
The replacement accepts new writes, but the ledger rejects recovery because the
insertion is gone. A former-owner variant alternates independent server copies
at the same endpoint and uses two pre-pinned clients. Both copies accept divergent
writes. This demonstrates absent distributed fencing without simulating two
simultaneously reachable owners or clearing either client's rollback anchors.
`begin_recovery_attempt` clears observations when changing the selected recovery
target; it retains every acknowledgment and the original fault time. A rejection
is recorded in the ledger instead of counting reads across branches as recovery.

The simultaneous-restore test defaults to four installations. Set
`FOKS_RECOVERY_TENANTS` to 1–32 for an explicit local experiment. Every source is
stopped and erased before parallel replacement startup, with a shared outer
15-second budget that includes teardown. Per-tenant clocks stop after successful
authenticated validation. This small fixture does not establish fleet density.

Set `FOKS_RECOVERY_EVIDENCE_DIR` to retain these new per-scenario JSONL ledgers.
Use a new directory for each run: existing filenames are rejected, not overwritten.
The original process-crash test continues to use `FOKS_RECOVERY_LEDGER` separately.

## Extending this into a hosted recovery qualification

Keep the controller and ledger outside the tested failure domain. Preserve
client stores, credentials, and HostID across every experiment. Adapt deployment
and fault injection to the actual replication platform before adding an abstract
provider interface. The same observation contract can then measure:

* Kill a writer immediately around commit/acknowledgment and lose the response;
  preserve every acknowledged write and reconcile ambiguous operations.
* Partition an old writer from its lease/storage peers while clients can reach
  it; promote another writer; resume the old writer. Reject stale-owner writes,
  including already queued work. A local file lock cannot establish this.
* Remove an entire AZ with the intended replica placement and quorum. Measure
  routing and client reconnect time through successful authenticated reads and
  writes, not merely a green health endpoint.
* Fail during revocation, key rotation, file publication and reclamation; check
  authorization and durable object reachability after recovery.
* Repeat with large metadata histories, simultaneous tenant activation, backups,
  disk/object-store delays, and exhausted admission queues.

Require zero loss of client-visible authoritative state, including newer roots
returned by reads. Bound replica lag **before** acknowledging or exposing that
state. Document which surviving quorum/network configuration supports successful
recovery within 15 seconds; an arbitrary partition cannot guarantee both
consistent writes and availability. A five-minute rewind is not an accepted
substitute and clearing client rollback anchors must not make the test pass.
