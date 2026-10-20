# M0 transaction boundary qualification

This spike compares the extracted PutDirents transaction with the unchanged native
Database API, then runs those vectors through the same Rust kernel compiled to Wasm
inside celld 0.6.2. Real initialized schema, constraints, triggers and exact encoded
dirents are used. Full-table copying happens only during fixture setup/assertion,
never while the kernel executes. It is **not a production backend**: current-user
authorization, native ownership fencing, production durable outcomes, full KV coverage and
end-to-end deadlines remain M1/M2/D work.

Equivalence applies to already initialized namespaces. One explicit negative vector
records an intentional difference: the old repository creates a new namespace before
its inner mutation transaction, so a later conflict retains that namespace; the
kernel rolls creation back with the failed command. Production adoption must review
this change as part of bootstrap/transaction ownership; the spike does not hide it.

Review the implementation before these gates. Use the shared Cargo slot and low
concurrency. Install `wasm32-unknown-unknown` before attempting the Wasm gate.

```sh
CARGO_INCREMENTAL=0 FOKS_METADATA_VECTORS=/private/tmp/foks-metadata-vectors \
  cargo test --locked -j2 -p foks-metadata-core --features native
CARGO_INCREMENTAL=0 RUSTFLAGS='-C link-arg=--max-memory=67108864' \
  cargo build --locked -j2 -p foks-metadata-core --lib --target wasm32-unknown-unknown
node --test tools/foks-metadata-spike/host.test.mjs
```

Invoke `run-local.py --help` for paths to the pinned celld binary, local Moto,
esbuild, built Wasm and generated catalog/vectors. The launcher uses a fresh local
bucket with dummy credentials and one fresh cell per vector. Results and exact
artifact hashes are retained. Managed S3 behavior, failover latency and production
capacity are not established by this experiment.

Commands are bounded to 2 MiB, dirent batches to 64, individual SQL payloads to
4 KiB and SQL calls to 2,048. A deliberately narrower feasibility profile accepts
at most 256 precondition directories/1,024 entries. Native production limits remain
unchanged. Integer parameters/results use decimal-text CASTs; JavaScript never
round-trips a SQLite integer through Number. The SQL catalog guards large stored
payloads in SQLite before V8 materializes them. Wasm is built with a 64 MiB linear
memory maximum; V8's heap is separately capped by the local launcher.

The host checks a one-second deadline between SQL calls. This is **not interruption
of a running SQLite statement or tight Wasm loop**, and does not establish the hard
request deadline gate. Statement query plans and remaining scan costs must be
reported before promoting this boundary. The original M0 `NameConflict` query
retained native historical-table behavior; its recorded cost below prompted the
separately reviewed schema56 current-head projection. Authority qualification uses
that projection from commit `295bc757`, not the older cost baseline.

Transaction errors and host/Wasm failures escape `transactionSync`, including
faults after the first real INSERT and deferred foreign-key failures at COMMIT.
No outcome is released on those paths; storage.sync runs after the transaction.
A trapped instance is discarded with its single-use qualification cell, not reused
as a production execution context. Seed data and invocation endpoints are private
qualification controls, never public FOKS APIs.

## Recorded M0 result (2026-10-15)

Against base `1d704a30` / server schema55 plus the isolated M0 manifest:
native tests3, Node host tests3 and Wasm-memory parser tests3 passed. Native
all-target and wasm32 library Clippy passed with warnings denied. The actual Wasm
artifact declared18 initial/1,024 maximum pages (1.125/64 MiB); observed memory
during each vector was1,245,184 bytes.

Pinned celld0.6.2 ran all17 vectors successfully in
`/private/tmp/foks-metadata-m0-celld-b/qualification.json`. The evidence records
artifact hashes, exact SQL outcome/state/reply equivalence and failures/traps.
Run A is retained separately: its state comparator incorrectly treated JSON object
key order as semantic. Both independent reviews approved canonical object-key
comparison plus negative exact-value tests before run B. No kernel change was needed.
These local-emulator runs do not measure production throughput or latency; another
component's Cargo work ran concurrently.

The native query-cost probe records a real prerequisite for the next phase:

| Retained matching-name versions | Unrelated current heads | SQLite VM steps |
| --- | --- | --- |
| 0 | 0 | 23 |
| 100 | 0 | 1,823 |
| 1,000 | 0 | 18,023 |
| 0 | 100 | 23 |
| 0 | 1,000 | 23 |
| 1,000 | 1,000 | 18,023 |

The plan selects `kv_dirent_listing(uid,parent_id,name_mac)` then joins current
heads. A current-head name projection is needed to remove retained-history work.
`/private/tmp/foks-metadata-m0-vectors/query-cost.json` contains the exact plans.
Pinned celld's SQLite progress interrupt hook is internal-test-only; its production
V8 CPU watchdog does not install SQLite cancellation. Cooperative host deadlines
therefore remain insufficient to certify interruption of one running SQL statement.
Production adoption also requires current authorization, owner epochs, recoverable
outcomes, bounded receipt retirement, complete route coverage and combined failover.

## Private authority fixture

The authority entry point executes the same Rust PutDirents kernel inside a host-owned
transaction. It adds controller/frontend capability separation, immutable installation
labels plus versioned activation, owner epoch and domain revision checks, principal/
namespace/policy binding, exact recoverable outcomes, and bounded intent retirement.
The configurable ceiling is 256 retained rows and 1 MiB reserved logical payload,
charging 4,608 bytes per intent (4,096-byte worst-case outcome plus fixed metadata).
Pruning and provably-unexecuted SQL-only settlement each process at most 64 rows.
Only the current epoch keeps an allocation high-water; closed epochs cannot execute.

These are qualification fixtures. Identity/key/artifact hashes are controller-supplied
labels and are not verified bootstrap or key retrievability evidence. Enabled fixture
principals may read namespace aggregate usage and their own outcomes; their stored role
controls PutDirents. Real FOKS credential/team authorization remains unimplemented here.
Optional normalization binds checked milliseconds-to-microseconds conversion at new
registration; retries retain the original time. No public protocol or production route
is enabled. No generic continuation rule is supplied for external side effects.

`FOKS_AUTHORITY_VECTORS=/private/tmp/<new-output>` on the native authority test exports
six independent suites: root, `errors`, `faults`, `settlement`, `bindings`, and
`normalization`, each with its own vectors and catalog. Run the existing launcher once
per suite with the built Wasm artifact. The launcher sends the wrong private capability
first and requires rejection before executing every authority vector. Each vector seeds
a fresh actual SQL fixture and compares complete resulting state, outcome and reply;
this is not a persistent native HTTP adapter or a node-loss experiment. Native-only
cases additionally exercise capacity, repeated epochs, malformed inputs and injected
infrastructure limits after a real domain effect. Native connection reopen checks
on-disk outcome recovery, not process/AZ recovery.

`node --test tools/foks-metadata-spike/durability.test.mjs` tests the host publication
boundary with controlled sync completion/failure. Pinned celld freezes Date and
performance clocks during synchronous execution. The helper checks admission, the
pre-COMMIT callback boundary, and refreshed time after sync, but cannot interrupt or
measure one synchronous SQL statement. A hanging sync can still hang this private
request; a late durable outcome is withheld and may be observed using a new bounded
lookup. Neither these checks nor emulator runs qualify a hard 15-second deadline.

The authority clock floor covers committed observations only. Rejected expiry rolls
back its floor update, so a trusted-time rollback can make pending execution eligible
again if its revision is unchanged. This fixture does not prove irreversible expiry;
the persistent service needs reviewed terminal expiry settlement or an external durable
monotonic floor, with a rejected-expiry/clock-rollback/retry regression.


## Recorded authority result (2026-10-15)

Against base `295bc757` / server schema56 plus the frozen17-file authority manifest:
11 native authority tests and3 existing differential tests passed. Node host/durability
13 tests passed, along with native all-target and wasm32 library Clippy with warnings
denied. The Wasm artifact declares18 initial/1024 maximum pages (1.125/64 MiB);
all actual celld vectors observed1,245,184 bytes of Wasm memory. These memory numbers
exclude the JS heap, SQL cache and fixture snapshots and are not tenant-density data.

Pinned celld0.6.2 passed all69 exact state/reply/outcome comparisons, and wrong private
capability checks were rejected for every vector:

| Suite | Vectors | Evidence |
| --- | ---: | --- |
| Authority/owner/revision/outcomes |28| `/private/tmp/foks-authority-celld-root-b/qualification.json` |
| Definite domain rejection |5| `/private/tmp/foks-authority-celld-errors-b/qualification.json` |
| SQL/receipt/trap/COMMIT faults |7| `/private/tmp/foks-authority-celld-faults-b/qualification.json` |
| Pending settlement |10| `/private/tmp/foks-authority-celld-settlement-b/qualification.json` |
| Canonical binding/activation |9| `/private/tmp/foks-authority-celld-bindings-b/qualification.json` |
| Trusted normalization/retry |10| `/private/tmp/foks-authority-celld-normalization-b/qualification.json` |

Each evidence file records immutable module/catalog/vector/host hashes. The final
native and lint logs are `/private/tmp/foks-authority-native-d.log`,
`foks-authority-native-clippy-c.log`, `foks-authority-wasm-clippy-b.log` and
`foks-authority-node-b.log`. All17 reviewed source hashes in
`/private/tmp/foks-authority-kernel-source-sha256.json` match main and the isolated
`metadata-authority-validation` tree. Other work used Cargo during runtime checks;
these are semantic qualifications, not controlled latency measurements.

Failures were preserved and corrected before rerunning. Native run A exposed serde's
unknown-field behavior for an internally tagged unit variant; an empty struct variant
keeps the valid JSON identical and rejects spoofed extra fields. Actual celld root A
passed27/28 but rejected raw SAVEPOINT SQL. The reviewed correction uses supported
nested `storage.transactionSync` around the same shared domain kernel. Reentrant Wasm
buffers and aggregate deadline/call budgets remain bounded; only a decoded domain
export's exact allowlisted rejection is converted after nested rollback. Imported
errors and traps abort the outer transaction. Final commit/sync still gates output.
Native and host tests cover exact2048/2049 logical-call boundaries, including control
charges, arbitrary same-code imported exceptions, rollback failure and memory growth.
No celld internals, public protocol changes or production listener mode were added.

## A1 personal qualification gate

The distinct personal exports use current issued-certificate/device/role rows;
the original synthetic principal fixture remains separate. Actual Rust and pinned
Go account fixtures export immutable vectors after completing native public account
and encrypted KV operations. The Worker still imports a fresh per-vector snapshot;
this is not a persistent production metadata service or a failover test.

After review and the exclusive Cargo slot are granted, generate vectors with
`FOKS_PERSONAL_VECTORS=/private/tmp/foks-personal-vectors` and
`cargo test --locked -j2 -p foks-server-testkit --features celld-proof --test personal_authority`.
Run its mandatory ignored Go gate separately with `FOKS_GO_ORACLE_DIR` pointing to
the prepared pinned oracle and `-- --ignored`; missing configuration fails that
explicit gate. Launch `run-local.py` separately for each exported `rust`/`go`
subdirectory using the freshly built Wasm artifact and matching catalog.

The original A1 gate covered scalar root/directory/small-node reads, internal
namespace quota counters, and bounded PutDirents. Group U additionally qualifies
public KvUsage through its separate PublicUsage point; QuotaCounters remains a
different internal accounting result. Group U schema58 qualification passed150 actual celld vectors (64 personal,17
differential,69 authority),26 shared native tests and29 Node/actual-Wasm tests;
see `/private/tmp/foks-usage-final-qualification.json` for frozen artifact evidence. Stale-cache
error vectors retain exact Snowpack bytes in canonical hex with an explicit64KiB
envelope bound; overflow reports qualification-only UnsupportedProfile. Large
public List/precondition/error responses, complete identity mutation coverage,
production authentication of frontend capability, artifact/key bootstrap and
successor original-context retrieval remain separate gates. The prior69-vector
authority implementation was committed as `f30c70c8`.

A1 validation on schema56 passed24 native core tests,5 parser tests, the live Rust
fixture, the explicitly enabled pinned Go fixture,14 Node tests, and96 actual
celld vectors (29 Rust personal,27 Go personal,40 prior authority/error/fault
regressions). Native and Wasm lint passed. The source/artifact audit and detailed
limits are recorded in `crates/foks-server/HOSTED_METADATA_PERSONAL_AUTH_PLAN.md`
and `/private/tmp/foks-personal-final-qualification.json`. These are semantic
qualification results, not persistent service or recovery-latency measurements.

The A2b page qualification uses a separate private binary ABI and pinned page
catalog. After the reviewed native/Wasm builds, export the actual schema vectors
with `FOKS_PAGE_VECTORS=/private/tmp/foks-page-vectors` while running the
`page_catalog` test target with `--features native`. Run the actual-Wasm tests with
`FOKS_PAGE_WASM`, `FOKS_PAGE_VECTORS` (the `vectors.json` file), and
`FOKS_PAGE_CATALOG` (the `page-catalog.json` file) set, using
`node --test tools/foks-metadata-spike/page-codec.test.mjs`. The real-celld launcher
accepts the same vector/scalar-catalog inputs plus
`--page-catalog /private/tmp/foks-page-vectors/page-catalog.json`.

Page result envelopes are bounded to 1 MiB and at most 64 rows; current-list rows
with optional maximum bodies use a computed limit of 15. Native allocation
admission is cumulative; the JS `page_declared_working_peak_bytes` field measures
a declared per-invocation working allowance, not allocator RSS or a node-wide
pool. Wasm memory is recorded separately. The private fixture permits a
65,537-byte stored node solely to check suppression of legacy oversized data;
the result payload cap remains 65,536. SQL exceptions/traps poison the page kernel
instance, requiring replacement; ordinary returned Rust errors do not. These
fixtures do not enable a public List route or prove a persistent metadata service,
credential/revision coverage, synchronous-SQL interruption, or HA recovery.


A2b commit19ac886f qualification on2026-10-16 passed40 native tests,37 Node/actual-Wasm tests,
strict native/Wasm lint and112 actual-celld vectors:26 binary pages plus86 scalar
and authority regressions. Evidence/source hashes are retained in
`/private/tmp/foks-a2b-final-qualification.json` and
`/private/tmp/foks-a2b-validation-sha256.json`. Maximum observed page Wasm memory
was1,703,936 bytes; the maximum declared JS page working allowance was1,266,750
bytes. Neither includes engine RSS or fixture snapshots. Production routes,
physical admission lifetimes, complete auth/revision hooks and hard SQL deadlines
remain unqualified by this private codec experiment.
