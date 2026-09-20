# Desktop notification acceptance benchmark

Run from the repository root with Node dependencies installed:

```sh
PCSC_LIB_DIR=/tmp/foks-pcsc-build PKG_CONFIG_PATH=/tmp/foks-pcsc-build \
  cargo build --release -p foks-agent --bin foks-agent \
  -p foks-server-testkit --example chat_notification_bench
python3 scripts/benchmarks/run-chat-notifications.py \
  --output /tmp/notification-current.jsonl
```

Use the local PC/SC development configuration appropriate for your platform. The
worker uses `TestEnvironment::ProductionBenchmark`: a real, isolated Rust server,
a separate real agent process, private temporary client state, and no OS keyring or
production account. Each arm gets a fresh fixture. The Python driver runs trials
sequentially, alternates arm order, refuses to overwrite results, and checkpoints
each completed trial. Do not run compilers or other benchmarks alongside acceptance.

The driver runs three repetitions per arm for 70-channel steady traffic, a
70-channel concentrated backlog, 200-channel wide traffic, and 70-channel slow
notification history with one permanent retryable failure. Each trial establishes
initial baselines, then runs 15 seconds of warmup, 60 seconds of measurement and a
fixed 10-second drain. Foreground history arrives every 500 ms; foreground message
workflows every two seconds; a second account sends six incoming messages per
second. The backlog adds 120 sequentially confirmed incoming sends at measurement
start, concurrently with ordinary traffic. These rates are explicit harness inputs,
not a claim to reconstruct unspecified rates from the supplied earlier benchmark.
An initial diagnostic at five reads and one write per second saturated the native
transport before the TCP buffering fix; those smoke results are not acceptance.

Foreground mutations use prepare/attempt/finalize through production scheduling.
The generator uses a verified team key, authenticated Basic `Send`, encrypted
synthetic text, and confirmed per-channel predecessor receipts. Its connection
opens lazily after fixture setup. One channel is reserved for foreground writes;
incoming traffic rotates through the remaining channels (or channel 1 for backlog).
The receiver uses production `ChatInboxService`, `NotificationConsumer`, scheduling,
TypeScript decoding, native desktop reply validation and real agent history.
`poll-inbox` retains its production bypass and separate account owner. The receiver
uses a ready snapshot with current service, compatibility, and inventory fields.
Setup checks committed consumer progress, including baselines seeded from verified
inbox positions without a history RPC; this read-only inspection ends before warmup.

Slow history adds 250 ms **after admission**, before the real native request. The
first channel then fails retryably on every attempt; later channels still use
real history. Preemption is disabled because IPC closure does not acknowledge agent
profile-lock release. There must be no queued background convoy ahead of foreground;
one already running RPC can still delay foreground admission.

## Baseline and diagnostics

```sh
python3 scripts/benchmarks/run-chat-notifications.py \
  --implementation baseline --output /tmp/notification-baseline.jsonl
```

Baseline mode extracts TypeScript from `6031904` into a private disposable directory.
That revision extracted the original FIFO without changing its behavior. Two
measurement-only hooks observe history admission and verified candidate IDs; neither
changes scheduling, filtering, progression or display. Native binaries are shared
between baseline and current TypeScript to isolate notification behavior. In
particular, both use the same TCP_NODELAY transport setting. This is an executable
scheduling/discovery baseline, not an old entire-stack binary comparison.

`--smoke --repetitions 1 --cases steady` shortens the driver's phases to 1/3/2 seconds.
The TypeScript entry point additionally supports `--channels`, `--worker`, `--agent`,
`--warmup`, `--duration`, and `--drain` for diagnosis. Changed dimensions or shortened
runs are **not** acceptance results. Worker and agent hashes and the harness hash
are recorded with each complete matrix trial.

## Comparing a later server and UI change

The legacy `--implementation baseline` always selects `6031904`. To compare a
later change against its immediate parent, supply that parent's source archive
and separately preserved release binaries instead:

```sh
python3 scripts/benchmarks/run-chat-notifications.py \
  --source /tmp/parent-source --source-revision <parent-commit> \
  --worker /tmp/parent-chat-notification-bench --agent /tmp/parent-foks-agent \
  --fault-onset measurement --output /tmp/notification-parent.jsonl
python3 scripts/benchmarks/run-chat-notifications.py \
  --worker /tmp/changed-chat-notification-bench --agent /tmp/changed-foks-agent \
  --fault-onset measurement --output /tmp/notification-changed.jsonl
```

Build each worker and agent from the corresponding revision before running either
matrix. Preserve the executables before rebuilding in a shared Cargo target;
never infer the linked server revision from the current checkout. The source
archive needs a `node_modules` symlink to the installed dependencies. For a Git
checkout, omit `--source-revision` to read its HEAD. With an archive, the explicit
revision records its provenance and must name the commit used to create it.
The driver hashes the selected binaries, harness, UI source files and chat limits.
Both result sets use `implementation: current` because both run the modern
notification design; `sourceRevision` and artifact hashes distinguish them.
Evaluate each full matrix with the summary script, then compare matching cases
and arms across revisions. Explicit sources cannot be combined with the legacy
baseline or its `--compare` mode.

## Interpreting results

- `foregroundHistory` measures successful explicit history operations from submission
  through completion. `execution` separates native execution by request class;
  `queueWait` measures explicit admission wait. `scheduler` includes all observed
  serialized outcomes, separately for foreground/background in the current scheduler.
- `confirmedSends` counts measured incoming sends with an actual server receipt,
  including late completions. `discoveredCandidates` matches verified candidates
  against those IDs by the drain deadline. `undiscoveredSends` stays in the result;
  candidate-delay percentiles describe discovered messages only. Disabled arms are
  expected to discover none. Backlog catch-up may intentionally skip predecessors
  under the two-page/100-message best-effort policy.
- `sinkCalls` counts attempted native displays, including bounded summaries and drain
  delivery. It is not a substitute for candidate discovery. The sink records no text.
- `busy` and `errors` classify measured requests, including polling. `sendErrors`
  identifies unconfirmed generator attempts. No retry is inferred from ambiguity.
- `retries` counts failure transitions scheduling another attempt in the current
  consumer. Baseline retry transitions are unavailable; its permanent-fault attempts
  remain visible in `faultCalls`. `cancellations` observes consumer cancellation
  outcomes, not every disposed view. `preemptions` is zero by design.
- `peakJobs` / `peakPerProfileJobs` include cancelled but unsettled current-consumer
  permits. This workload has one receiving profile; deterministic tests cover two
  profiles and replacement consumers. Baseline job metrics are null rather than
  inferred from RPC overlap; `peakBackgroundRPCs` is observed in both modes.
- `foregroundSlots` counts measured read arrivals; writes also contribute to
  `missedSlots` if the bounded admission limit is reached. A missed slot is a failure.
- `lateDisplays` must remain zero. RSS is sampled separately for Node, the Rust worker
  (which hosts the isolated server), and the real agent. Event-loop p95 uses 20 ms
  sampling. Metadata stays volatile and outputs omit scope IDs and message text.

Compare the median per-trial successful foreground-history p95 across each arm.
Report both relative and absolute changes; the combined review trigger is an
increase exceeding **both 20% and 50 ms** for steady, backlog or wide traffic.
For enabled steady and wide arms, the provisional discovery target remains >=99%
of measured confirmed sends by the fixed drain. A failure is not waived by good
percentiles. Every enabled slow trial must execute the fault and continue healthy
work. Review failures before changing any objective.

For a controlled slow comparison, select the permanent key from the first measured
notification history request. Warmup retry timing then cannot prevent the required
fault from executing. The chosen key stays fixed for the remainder of the trial:

```sh
python3 scripts/benchmarks/run-chat-notifications.py --compare \
  --cases slow --arms on --fault-onset measurement \
  --output /tmp/notification-slow-paired.jsonl
```

The driver refuses to accept a completed slow trial with no fault or no healthy
continuation. The original `--fault-onset setup` mode remains available to reproduce
the primary matrix. Raw source/artifact hashes accompany the paired comparisons.
Run `python3 scripts/benchmarks/test_summary.py` to check the result validator and
`python3 scripts/benchmarks/summarize-chat-notifications.py <current-results.jsonl>`
to evaluate a complete primary matrix; partial data never passes acceptance.

## Inbox CPU benchmark

```sh
node --import tsx scripts/benchmarks/inbox-publication.ts --source /tmp/parent-source
node --import tsx scripts/benchmarks/inbox-publication.ts
```

At 10, 100 and 1,000 channels, `sync` measures full-reply publication, `read`
measures a confirmed single-channel read, and `revision` measures a refresh-map
update. Each includes one subscriber scanning conversations. The additional
`full-sync` workload runs the full UI sync method with an immediate bridge,
including profile scheduling, authority checks, preview filtering, both revision
calculations, publication, and the subscriber scan. It checks that all 520 warmup
and measured requests publish ready data, so failed requests cannot appear faster.
It uses a healthy inbox; the revision-only workload represents a degraded refresh.
Network/native RPC time and rendering are excluded. Each result is the median of
five batch means of 100 operations after 20 warmups, in microseconds. Run these
measurements without concurrent compilers or acceptance trials.
