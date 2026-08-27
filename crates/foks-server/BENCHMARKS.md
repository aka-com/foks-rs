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
