# FOKS server workload benchmarks

The opt-in benchmark gate exercises the real loopback TLS/RPC server in release
mode. It reports observations for the machine running it; it is not a portable
capacity claim or a substitute for a deployment-specific soak test.

Run all three workloads from the repository root:

```sh
tools/foks-server/bench.sh
```

The workloads cover:

- 128 simultaneous public-listener TLS connections and RPC responses, equal to
  the benchmark profile's active-session limit;
- 48 persistent clients withholding most of a frame while 32 healthy clients
  use the same listener and its single configured runtime worker;
- 10,000 attempts against a deliberately full SQLite writer queue, followed by
  a drain-and-recovery assertion.

Each `BENCH` line records sample/error counts, wall time, operations per second,
p50/p95/p99 latency in nanoseconds, and available hardware parallelism. The test configuration,
durability PRAGMAs, dataset, Git revision, OS, CPU model, database/WAL size, and
checkpoint cost must be captured separately for any release capacity report.

The assertions intentionally cover correctness: bounded rejection, healthy
client progress, and recovery. Absolute latency thresholds belong in a stable,
dedicated deployment environment and should not be inferred from developer or
shared CI machines.


## WAL checkpoint comparison

The opt-in bundled-SQLite comparison uses the real `Writer` queue and the same
maintenance implementation as manual and periodic passes. It compares TRUNCATE
with unrestricted retention, PASSIVE with unrestricted retention, and PASSIVE
with the proposed 16 MiB default, on separate disposable databases:

```sh
cargo test --locked -j 2 --release -p foks-server --lib compare_checkpoint_workloads -- --ignored --nocapture
```

Each variant keeps FULL synchronous mode and the normal 1,000-page automatic
checkpoint threshold. A 250 ms busy timeout bounds the benchmark's deliberate
TRUNCATE stalls; production defaults remain five seconds. Unrestricted retention
uses SQLite's maximum signed-integer limit, equivalent for these small datasets
to the former unlimited policy. The workload seeds 32 MiB of opaque log-send
blocks while a read transaction is pinned, then runs four phases with four
concurrent writers, 256 small name reservations, and eight maintenance passes:

1. A held reader through the entire phase.
2. Catch-up after releasing that reader.
3. Two overlapping, repeatedly renewed short read transactions.
4. An online SQLite backup alongside writes and maintenance.

`CHECKPOINT_BENCH` JSON lines record checkpoint execution time, report outcomes,
frame positions, timed physical WAL samples, writer queue-wait and write-latency
p50/p95/p99/max, throughput, SQLite version and connection settings. Completion,
natural reuse, backup payload size/integrity, persisted write counts, and restart
integrity are checked. The held-reader regression in the ordinary unit suite
separately requires maintenance and a following write to finish before releasing
the reader, with a cleanup-safe watchdog shorter than the busy timeout.

Record the command, Git revision, machine/CPU and build profile alongside output.
Run repeated release-mode comparisons on deployment-representative storage before
making capacity or tail-latency claims. These synthetic workloads establish the
reader-wait mechanism and expose remaining checkpoint I/O; they do not prove that
16 MiB is optimal or that PASSIVE imposes a hard writer-latency bound.

### Local observations, 2026-09-20

Three optimized runs on an Apple M5 (10 hardware threads), macOS 26.5.1,
aarch64, Rust 1.95.0, bundled SQLite 3.53.2; implementation based on parent `f5b45b43`.
The full test workload had ended before these measurements. This is a small
synthetic comparison on a developer machine, not a release capacity result.
Ranges below are the minimum and maximum across those three runs.

| Phase | Checkpoint / retention | Queue p99 (ms) | Queue max (ms) | Writes/s |
| --- | --- | ---: | ---: | ---: |
| held-reader | truncate-unlimited | 288.27–290.48 | 290.18–290.94 | 109–110 |
| held-reader | passive-unlimited | 1.05–1.53 | 1.09–1.58 | 2835–3786 |
| held-reader | passive-16mib | 1.07–6.09 | 1.19–12.89 | 2951–3654 |
| catch-up | truncate-unlimited | 1.44–1.87 | 19.74–28.18 | 5627–6865 |
| catch-up | passive-unlimited | 0.37–0.91 | 40.00–80.06 | 2519–4819 |
| catch-up | passive-16mib | 1.11–5.82 | 34.41–44.38 | 4337–4707 |
| overlapping-readers | truncate-unlimited | 1.90–2.54 | 1.99–2.77 | 5173–8505 |
| overlapping-readers | passive-unlimited | 0.53–3.89 | 0.58–5.13 | 5562–11899 |
| overlapping-readers | passive-16mib | 0.49–1.51 | 0.58–1.53 | 10374–12167 |
| online-backup | truncate-unlimited | 1.98–2.56 | 4.71–5.46 | 9285–11420 |
| online-backup | passive-unlimited | 0.98–3.67 | 3.58–4.29 | 8088–14072 |
| online-backup | passive-16mib | 0.84–3.25 | 1.22–3.29 | 12764–16030 |

With the reader held, the proposed policy removed the busy-timeout-sized stall:
TRUNCATE checkpoint maxima were 286–288 ms, versus 0.18–0.28 ms for PASSIVE with
16 MiB retention. PASSIVE reported deferred progress while the reader remained
pinned. After release, confirmed completion and natural reuse succeeded;
unrestricted PASSIVE retained 39,243,032 bytes versus 16,777,216 bytes with the
reuse limit. All runs passed backup and restart integrity checks.

Catch-up still occupied the writer: observed queue maxima with the proposed
policy reached 44 ms. Automatic checkpoints remain enabled and may perform that
I/O on a committing write before the explicit checkpoint executes. Do not read
a short explicit-checkpoint duration as a bound on writer latency.

The unrestricted PASSIVE control and 16 MiB variant show overlapping throughput
ranges and noisy tails, including a slower held-reader sample with the limit.
These runs do not establish a universal performance improvement or a
statistically bounded absence of regressions from the reuse limit. Keep the
16 MiB policy as a measured local starting point and repeat representative
release/soak measurements before deployment-specific sizing decisions.
