/** Real-process acceptance harness; synthetic data and volatile message IDs only. */
import { spawn, execFileSync } from 'node:child_process';
import { createInterface } from 'node:readline';
import {
  mkdtempSync,
  readFileSync,
  writeFileSync,
  symlinkSync,
  rmSync,
} from 'node:fs';
import { resolve, join } from 'node:path';
import { tmpdir } from 'node:os';
import { pathToFileURL } from 'node:url';
import { monitorEventLoopDelay } from 'node:perf_hooks';
import type { Bridge } from '../../apps/desktop/src/bridge';
import type {
  ChatAction,
  ChatScope,
} from '../../apps/desktop/src/chat-contract';
import type { NotificationMetric } from '../../apps/desktop/src/chat/notification-consumer';
import type { ChatInboxTiming } from '../../apps/desktop/src/chat/inbox-service';
import { notificationBenchmarkSnapshot } from './chat-notification-fixture';
import { decodeChatScope } from '../../apps/desktop/src/chat-contract';
import type { WorkTiming } from '../../apps/desktop/src/scheduling/profile-work';

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw Error('Invalid benchmark worker response');
  return value as Record<string, unknown>;
}

interface WorkerReady {
  channels: string[];
  scope: ChatScope;
  agentPid: number;
}

function recordResponse(value: unknown): { messageId: string } {
  const response = record(value);
  if (typeof response.messageId !== 'string' || response.messageId.length === 0)
    throw Error('Invalid benchmark send response');
  return { messageId: response.messageId };
}

function decodeReady(value: Record<string, unknown>): WorkerReady {
  if (
    !Array.isArray(value.channels) ||
    !value.channels.every(
      (channel): channel is string => typeof channel === 'string',
    ) ||
    typeof value.agentPid !== 'number' ||
    !Number.isSafeInteger(value.agentPid) ||
    value.agentPid <= 0
  )
    throw Error('Invalid benchmark readiness response');
  const store = record(record(value.scope).store);
  const storeId = JSON.stringify({
    kind: 'team',
    profile: store.profile,
    accountAlias: store.account_alias,
    teamAlias: store.team_alias,
    teamId: store.team_id,
  });
  return {
    channels: value.channels,
    scope: decodeChatScope(value.scope, storeId),
    agentPid: value.agentPid,
  };
}

const args = process.argv.slice(2);
const option = (name: string, fallback: string) => {
  const i = args.indexOf(`--${name}`);
  return i < 0 ? fallback : args[i + 1];
};
const workload = option('case', 'steady');
const faultOnset = option('fault-onset', 'setup');
if (!['setup', 'measurement'].includes(faultOnset))
  throw Error('Invalid fault onset');
if (!['steady', 'backlog', 'wide', 'slow'].includes(workload))
  throw Error('Invalid workload');
const enabled = option('notifications', 'on') === 'on',
  baseline = option('implementation', 'current') === 'baseline';
const warmup = Number(option('warmup', '15')) * 1000,
  duration = Number(option('duration', '60')) * 1000,
  drain = Number(option('drain', '10')) * 1000;
if (
  ![warmup, duration, drain].every(
    (n) => Number.isFinite(n) && n > 0 && n <= 120000,
  )
)
  throw Error('Invalid trial duration');
const root = resolve('.');
let source = resolve(option('source', root)),
  scratch: string | undefined;
process.on('exit', () => {
  if (scratch) rmSync(scratch, { recursive: true, force: true });
});
if (baseline && source !== root)
  throw Error('Choose either an explicit source or the legacy baseline');
if (baseline) {
  scratch = mkdtempSync(join(tmpdir(), 'foks-notification-baseline-'));
  execFileSync('tar', ['-x', '-C', scratch], {
    input: execFileSync(
      'git',
      [
        'archive',
        '6031904',
        'apps/desktop/src',
        'package.json',
        'crates/foks-agent-proto/chat-limits.json',
      ],
      { maxBuffer: 32 * 1024 * 1024 },
    ),
  });
  symlinkSync(join(root, 'node_modules'), join(scratch, 'node_modules'), 'dir');
  const file = join(scratch, 'apps/desktop/src/chat/notification-consumer.ts');
  const text = readFileSync(file, 'utf8');
  const marker =
    '      // Commit local progress before OS effects: ambiguous delivery is not retried.';
  if (!text.includes(marker))
    throw Error('Baseline instrumentation anchor changed');
  writeFileSync(
    file,
    text.replace(
      marker,
      '      (globalThis as any).__foksBenchMetric({kind: "pass", candidates: candidates.map(m => m.id), baselineOnly: false, incomplete});\n' +
        marker,
    ),
  );
  const clientFile = join(scratch, 'apps/desktop/src/chat/client.ts');
  const clientSource = readFileSync(clientFile, 'utf8');
  const enqueueMarker = '    const work = async () => {';
  if (!clientSource.includes(enqueueMarker))
    throw Error('Baseline admission anchor changed');
  writeFileSync(
    clientFile,
    clientSource.replace(
      enqueueMarker,
      '    (globalThis as any).__foksBenchEnqueue?.(action);\n' + enqueueMarker,
    ),
  );
  source = scratch;
}
const load = (name: string) =>
  import(pathToFileURL(join(source, `apps/desktop/src/${name}.ts`)).href);
const { ChatInboxService } = await load('chat/inbox-service');
const { NotificationConsumer } = await load('chat/notification-consumer');
const { chatClient } = await load('chat/client');
const { decodeChatReply } = await load('chat-contract');
const { observeProfileWork } = await load('scheduling/profile-work');
Object.defineProperty(globalThis, 'document', {
  value: { visibilityState: 'hidden', hasFocus: () => false },
  configurable: true,
});
const delay = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const channelsCount = Number(
  option('channels', workload === 'wide' ? '200' : '70'),
);
if (
  !Number.isInteger(channelsCount) ||
  channelsCount < 2 ||
  channelsCount > 200
)
  throw Error('Invalid channel count');
const worker = spawn(
  resolve(option('worker', 'target/release/examples/chat_notification_bench')),
  [
    resolve(option('agent', 'target/release/foks-agent')),
    String(channelsCount),
  ],
  { stdio: ['pipe', 'pipe', 'inherit'] },
);
let readyResolve!: (value: WorkerReady) => void,
  readyReject!: (error: Error) => void;
const ready = new Promise<WorkerReady>((r, j) => {
  readyResolve = r;
  readyReject = j;
});
const requests = new Map<
  number,
  { resolve(value: unknown): void; reject(error: unknown): void }
>();
let requestId = 0,
  stopped = false;
worker.on('exit', (code) => {
  const error = Error(`Benchmark worker exited (${code})`);
  readyReject(error);
  for (const request of requests.values()) request.reject(error);
  requests.clear();
});
createInterface({ input: worker.stdout }).on('line', (line) => {
  const value = record(JSON.parse(line));
  if (value.ready === true) readyResolve(decodeReady(value));
  else {
    if (typeof value.id !== 'number' || !Number.isSafeInteger(value.id))
      throw Error('Invalid benchmark response ID');
    const request = requests.get(value.id);
    requests.delete(value.id);
    if (value.error) request?.reject(value.error);
    else request?.resolve(value.value);
  }
});
const rpc = (value: object) =>
  new Promise<unknown>((resolve, reject) => {
    if (requests.size >= 24)
      return reject(Error('Benchmark admission overflow'));
    const id = ++requestId;
    requests.set(id, { resolve, reject });
    worker.stdin.write(JSON.stringify({ ...value, id }) + '\n');
  });
let cleanup: (() => void) | undefined;
try {
  const info = await ready;
  let faultKey: string | undefined =
    faultOnset === 'setup' ? info.channels[0] : undefined;
  const storeId = JSON.stringify({
    kind: 'team',
    profile: 'receiver',
    accountAlias: 'receiver',
    teamAlias: 'bench',
    teamId: info.scope.store.team_id,
  });
  let measured = false,
    measurementStart = Infinity,
    measurementEnd = Infinity,
    sinkCalls = 0,
    lateDisplays = 0,
    faultCalls = 0,
    healthyAfterFault = 0,
    retries = 0,
    cancellations = 0,
    incomplete = 0,
    peakJobs = 0,
    peakBackgroundRPCs = 0,
    backgroundRPCs = 0,
    foregroundSlots = 0,
    missedSlots = 0,
    sendErrors = 0;
  const seen = new Map<string, number>(),
    confirmed = new Map<string, number>(),
    baselined = new Set<string>();
  const timings: {
    priority: string;
    queueMilliseconds: number;
    executionMilliseconds: number;
    outcome: string;
  }[] = [];
  const foreground: number[] = [],
    execution: Record<string, number[]> = {},
    queueWait: Record<string, number[]> = {},
    busy: Record<string, number> = {},
    errors: Record<string, number> = {};
  const queued = new WeakMap<object, { time: number; kind: string }>();
  const metric = (event: NotificationMetric) => {
    if (event.kind === 'state') peakJobs = Math.max(peakJobs, event.active);
    if (event.kind === 'failure') {
      retries += Number(!event.cancelled);
      cancellations += Number(event.cancelled);
    }
    if (event.kind === 'pass') {
      incomplete += Number(event.incomplete);
      for (const id of event.candidates)
        if (!seen.has(id)) seen.set(id, performance.now());
    }
  };
  const instrumentation = globalThis as typeof globalThis & {
    __foksBenchMetric?: (event: NotificationMetric) => void;
    __foksBenchEnqueue?: (action: ChatAction) => void;
  };
  instrumentation.__foksBenchMetric = metric;
  instrumentation.__foksBenchEnqueue = (action: ChatAction) => {
    if (action.action === 'history' && !queued.has(action))
      queued.set(action, {
        time: performance.now(),
        kind: 'notification-history',
      });
  };
  const bridge = {
    chat: async (id: string, action: ChatAction, view: string) => {
      const explicit = queued.get(action);
      const notification =
        action.action === 'notification-history' ||
        (baseline &&
          action.action === 'history' &&
          explicit?.kind !== 'foreground-history');
      const kind = notification
        ? 'notification-history'
        : (explicit?.kind ?? action.action);
      const start = performance.now(),
        record = measured;
      if (record && explicit)
        (queueWait[kind] ??= []).push(start - explicit.time);
      if (notification) {
        backgroundRPCs++;
        peakBackgroundRPCs = Math.max(peakBackgroundRPCs, backgroundRPCs);
      }
      try {
        if (notification && workload === 'slow') {
          // Choose once from actual measured work; warmup backoff cannot hide the fault.
          if (
            faultOnset === 'measurement' &&
            record &&
            !faultKey &&
            'channel' in action
          )
            faultKey = action.channel;
          await delay(250);
          if ('channel' in action && action.channel === faultKey) {
            faultCalls++;
            throw {
              code: 'offline',
              message: 'Injected retryable failure.',
              retryable: true,
              ambiguous: false,
              fatal: false,
            };
          }
        }
        const result = decodeChatReply(
          await rpc({ kind: 'chat', action, view }),
          id,
          action,
        );
        if (notification && 'channel' in action) {
          baselined.add(action.channel);
          if (faultCalls) healthyAfterFault++;
        }
        return result;
      } catch (error: unknown) {
        if (record) {
          errors[kind] = (errors[kind] ?? 0) + 1;
          if (
            error &&
            typeof error === 'object' &&
            'code' in error &&
            error.code === 'profile-busy'
          )
            busy[kind] = (busy[kind] ?? 0) + 1;
        }
        throw error;
      } finally {
        if (record) (execution[kind] ??= []).push(performance.now() - start);
        if (notification) backgroundRPCs--;
      }
    },
    cancelChat: async (view: string) => {
      await rpc({ kind: 'cancel', view });
    },
    chatLocal: async (action: Parameters<Bridge['chatLocal']>[0]) => {
      if (action.action === 'display') {
        if (stopped) lateDisplays++;
        if (measured) sinkCalls++;
      }
    },
  } as unknown as Bridge;
  const off = observeProfileWork(bridge, (event: WorkTiming) => {
    if (measured) timings.push(event);
  });
  const service = new ChatInboxService(bridge);
  const snapshot = notificationBenchmarkSnapshot(storeId, info.scope);
  service.updateStores(
    baseline
      ? {
          ...snapshot,
          servers: snapshot.servers.map((server) => ({
            ...server,
            chat_available: true,
          })),
        }
      : snapshot,
  );
  const stopSetupDiagnostics =
    typeof service.observe === 'function'
      ? service.observe((event: ChatInboxTiming) => {
          if (event.kind === 'sync' && event.outcome !== 'ok')
            console.error(
              `benchmark setup sync: ${event.outcome} ${event.code ?? 'unknown'}`,
            );
        })
      : () => {};
  service.start();
  const consumer = enabled
    ? new NotificationConsumer(
        bridge,
        service,
        {
          epoch: 'aa'.repeat(16),
          available: true,
          settings: { enabled: true, previews: false, overrides: {} },
        },
        () => {},
        undefined,
        metric,
      )
    : undefined;
  cleanup = () => {
    stopped = true;
    stopSetupDiagnostics();
    consumer?.stop();
    service.stop();
    off();
  };
  // Modern consumers can seed from verified inbox positions without an RPC.
  // Read committed progress only during setup; never mutate consumer metadata.
  const initializedChannels = (): ReadonlySet<string> => {
    if (baseline || !consumer) return baselined;
    const progress = (
      consumer as unknown as {
        progress: Map<
          string,
          {
            channel: string;
            baseline: bigint | null;
            baselineOnly: boolean;
          }
        >;
      }
    ).progress;
    return new Set(
      [...progress.values()]
        .filter(
          (p) => typeof p.baseline === 'bigint' && p.baselineOnly === false,
        )
        .map((p) => p.channel),
    );
  };
  const setupDeadline = performance.now() + 120000;
  while (true) {
    const initialized = initializedChannels();
    const expected = info.channels.filter(
      (channel) =>
        !(
          workload === 'slow' &&
          faultOnset === 'setup' &&
          channel === faultKey
        ),
    );
    const inbox = service.getSnapshot().get(storeId);
    if (
      inbox?.state === 'ready' &&
      !inbox.stale &&
      inbox.data &&
      (!enabled || expected.every((channel) => initialized.has(channel)))
    )
      break;
    if (performance.now() > setupDeadline)
      throw Error(
        `Baseline setup timed out (${initialized.size}/${channelsCount}; state=${inbox?.state ?? 'missing'}; code=${inbox?.failure?.code ?? 'none'})`,
      );
    await delay(50);
  }
  stopSetupDiagnostics();
  console.error('benchmark receiver: inbox and notification baselines ready');
  // Every trial begins with established baselines, then exactly 15 seconds of warmup.
  const histogram = monitorEventLoopDelay({ resolution: 20 });
  histogram.enable();
  let peakRSS = process.memoryUsage().rss,
    peakWorkerRSS = 0,
    peakAgentRSS = 0;
  const rss = (pid: number) => {
    try {
      return (
        Number(
          /VmRSS:\s+(\d+)/.exec(
            readFileSync(`/proc/${pid}/status`, 'utf8'),
          )?.[1] ?? 0,
        ) * 1024
      );
    } catch {
      return 0;
    }
  };
  const memory = setInterval(() => {
    peakRSS = Math.max(peakRSS, process.memoryUsage().rss);
    peakWorkerRSS = Math.max(peakWorkerRSS, rss(worker.pid!));
    peakAgentRSS = Math.max(peakAgentRSS, rss(info.agentPid));
  }, 1000);
  let pending = 0,
    sendIndex = 0,
    writeIndex = 0;
  const jobs = new Set<Promise<unknown>>();
  const track = (job: Promise<unknown>) => {
    jobs.add(job);
    void job.finally(() => jobs.delete(job));
  };
  const read = async () => {
    const client = chatClient(bridge, 'receiver', storeId),
      start = performance.now(),
      record = measured;
    const action: ChatAction = {
      action: 'history',
      channel: info.channels[sendIndex % channelsCount],
      before: null,
    };
    queued.set(action, { time: start, kind: 'foreground-history' });
    try {
      await client.request(action);
      if (record) foreground.push(performance.now() - start);
    } catch {
      // The bridge records failed RPCs; only successful reads contribute latency.
    } finally {
      client.dispose();
      pending--;
    }
  };
  const write = async () => {
    const client = chatClient(bridge, 'receiver', storeId);
    try {
      const prepare: ChatAction = {
        action: 'prepare-message',
        submission: (++writeIndex + 100000).toString(16).padStart(32, '0'),
        channel: info.channels[channelsCount - 1],
        text: 'foreground benchmark message',
      };
      queued.set(prepare, {
        time: performance.now(),
        kind: 'foreground-write',
      });
      const reply = await client.request(prepare);
      const attempt: ChatAction = {
        action: 'attempt',
        operation: reply.result.operation.id,
      };
      queued.set(attempt, {
        time: performance.now(),
        kind: 'foreground-write',
      });
      const done = await client.request(attempt);
      if (done.result.operation.state === 'confirmed')
        await client.request({
          action: 'finalize',
          operation: done.result.operation.id,
        });
    } catch {
      // The bridge records the error. Continue the measured workload without replay.
    } finally {
      client.dispose();
      pending--;
    }
  };
  const send = async (index: number, record: boolean) => {
    try {
      const result = await rpc({
        kind: 'send',
        channel:
          info.channels[
            workload === 'backlog' ? 1 : index % (channelsCount - 1)
          ],
      });
      const response = recordResponse(result);
      if (record) confirmed.set(response.messageId, performance.now());
    } catch {
      if (record) sendErrors++;
    }
  };
  let nextRead = performance.now(),
    nextWrite = nextRead,
    nextSend = nextRead;
  const workloadStart = nextRead;
  measurementStart = workloadStart + warmup;
  measurementEnd = measurementStart + duration;
  while (performance.now() < measurementEnd) {
    const now = performance.now();
    if (!measured && now >= measurementStart) {
      measured = true;
      faultCalls = 0;
      healthyAfterFault = 0;
      retries = 0;
      cancellations = 0;
      incomplete = 0;
      histogram.reset();
      if (workload === 'backlog')
        track(
          (async () => {
            for (let i = 0; i < 120; i++) await send(sendIndex++, true);
          })(),
        );
    }
    if (now >= nextRead) {
      if (measured) foregroundSlots++;
      if (pending >= 16) {
        if (measured) missedSlots++;
      } else {
        pending++;
        track(read());
      }
      nextRead += 500;
    }
    if (now >= nextWrite) {
      if (pending >= 16) {
        if (measured) missedSlots++;
      } else {
        pending++;
        track(write());
      }
      nextWrite += 2000;
    }
    if (now >= nextSend) {
      track(send(sendIndex++, measured));
      nextSend += 1000 / 6;
    }
    await delay(
      Math.max(
        1,
        Math.min(
          10,
          nextRead - performance.now(),
          nextSend - performance.now(),
        ),
      ),
    );
  }
  // No new traffic during the fixed drain; in-flight sends remain in the denominator.
  await delay(drain);
  const drainDeadline = performance.now();
  cleanup();
  clearInterval(memory);
  histogram.disable();
  await Promise.allSettled([...jobs]);
  await delay(300);
  const discovered = [...confirmed].filter(
    ([id]) => (seen.get(id) ?? Infinity) <= drainDeadline,
  );
  const latencies = discovered.map(([id, time]) =>
    Math.max(0, seen.get(id)! - time),
  );
  const stats = (values: number[]) => {
    const a = [...values].sort((a, b) => a - b);
    return {
      count: a.length,
      p50: a[Math.floor(a.length * 0.5)] ?? null,
      p95: a[Math.min(a.length - 1, Math.floor(a.length * 0.95))] ?? null,
      max: a.at(-1) ?? null,
    };
  };
  console.log(
    JSON.stringify({
      workload,
      implementation: baseline ? 'baseline-6031904' : 'current',
      enabled,
      warmupMs: warmup,
      durationMs: duration,
      drainMs: drain,
      channels: channelsCount,
      foregroundHistory: stats(foreground),
      execution: Object.fromEntries(
        Object.entries(execution).map(([k, v]) => [k, stats(v)]),
      ),
      queueWait: Object.fromEntries(
        Object.entries(queueWait).map(([k, v]) => [k, stats(v)]),
      ),
      scheduler: Object.fromEntries(
        ['foreground', 'background'].map((priority) => [
          priority,
          {
            queue: stats(
              timings
                .filter((t) => t.priority === priority)
                .map((t) => t.queueMilliseconds),
            ),
            execution: stats(
              timings
                .filter((t) => t.priority === priority)
                .map((t) => t.executionMilliseconds),
            ),
          },
        ]),
      ),
      confirmedSends: confirmed.size,
      discoveredCandidates: discovered.length,
      undiscoveredSends: confirmed.size - discovered.length,
      completeness: discovered.length / Math.max(1, confirmed.size),
      candidateDelay: stats(latencies),
      sinkCalls,
      lateDisplays,
      busy,
      errors,
      sendErrors,
      foregroundSlots,
      missedSlots,
      faultOnset,
      faultKeyIndex: faultKey ? info.channels.indexOf(faultKey) : null,
      faultCalls,
      healthyAfterFault,
      retries,
      cancellations,
      preemptions: 0,
      peakJobs: baseline ? null : peakJobs,
      peakBackgroundRPCs,
      peakPerProfileJobs: baseline ? null : peakJobs,
      incompletePasses: incomplete,
      eventLoopP95Ms: histogram.percentile(95) / 1e6,
      peakRSS,
      peakWorkerRSS,
      peakAgentRSS,
    }),
  );
} finally {
  cleanup?.();
  worker.stdin.end();
  await Promise.race([
    new Promise<void>((r) => worker.once('exit', () => r())),
    delay(5000).then(() => worker.kill('SIGTERM')),
  ]);
  if (scratch) rmSync(scratch, { recursive: true, force: true });
}
