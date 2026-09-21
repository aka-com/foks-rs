import assert from 'node:assert/strict';
import test from 'node:test';
import { formatTimings } from '../src/diagnostics/format';
import {
  DiagnosticLog,
  hashId,
  redactKey,
  type TimingClock,
  type TimingEvent,
} from '../src/diagnostics/log';
import { subscribeDiagnostics } from '../src/diagnostics/subscribe';
import {
  observeProfileWork,
  scheduleProfileWork,
} from '../src/scheduling/profile-work';
import { ReconciliationScheduler } from '../src/scheduling/reconciliation';

class Clock implements TimingClock {
  wall = 1_700_000_000_000;
  mono = 0;
  now = () => this.wall;
  elapsed = () => this.mono;
  tick(ms: number) {
    this.wall += ms;
    this.mono += ms;
  }
}

test('the log is bounded, ordered oldest first, and bounds every entry', () => {
  const clock = new Clock();
  const log = new DiagnosticLog(3, clock);
  for (let index = 0; index < 5; index++) {
    log.record({ name: `e${index}`, attrs: { index } });
    clock.tick(1);
  }
  assert.equal(log.length, 3);
  assert.deepEqual(
    log.events().map((event) => event.name),
    ['e2', 'e3', 'e4'],
  );
  const wide = log.record({
    name: 'x'.repeat(200),
    scope: 'y'.repeat(200),
    attrs: Object.fromEntries(
      Array.from({ length: 20 }, (_, index) => [`k${index}`, 'v'.repeat(200)]),
    ),
  });
  assert.equal(wide.name.length, 64);
  assert.equal(wide.scope?.length, 64);
  assert.equal(Object.keys(wide.attrs ?? {}).length, 8);
  assert.equal(String(wide.attrs?.k0).length, 64);
  assert.ok(Object.isFrozen(wide));
  log.clear();
  assert.equal(log.length, 0);
});

test('a span records once with the elapsed time and merged attributes', () => {
  const clock = new Clock();
  const log = new DiagnosticLog(8, clock);
  const end = log.span('invoke', { scope: 'srv', attrs: { command: 'c' } });
  clock.tick(120);
  end('error', { code: 'busy', attrs: { extra: true } });
  end('ok');
  assert.equal(log.length, 1);
  const [event] = log.events();
  assert.equal(event.ms, 120);
  assert.equal(event.outcome, 'error');
  assert.equal(event.code, 'busy');
  assert.deepEqual(event.attrs, { command: 'c', extra: true });
});

test('collect merges sources by time and survives a failing source', async () => {
  const clock = new Clock();
  const log = new DiagnosticLog(8, clock);
  clock.tick(10);
  log.record({ name: 'renderer-late' });
  const stop = log.addSource(async () => [
    { at: clock.wall - 5, layer: 'backend', name: 'backend-early' },
  ]);
  log.addSource(() => Promise.reject(new Error('unavailable')));
  assert.deepEqual(
    (await log.collect()).map((event) => event.name),
    ['backend-early', 'renderer-late'],
  );
  stop();
  assert.deepEqual(
    (await log.collect()).map((event) => event.name),
    ['renderer-late'],
  );
});

test('hashId is short, stable and never the input', () => {
  assert.equal(hashId('team-store-id'), hashId('team-store-id'));
  assert.notEqual(hashId('a'), hashId('b'));
  assert.match(hashId('anything'), /^[0-9a-f]{6}$/);
});

test('the scheduler and the profile queue report into the log with their scope', async () => {
  const clock = new Clock();
  const log = new DiagnosticLog(64, clock);
  const timers: (() => void)[] = [];
  const scheduler = new ReconciliationScheduler({
    now: () => clock.wall,
    later: (run) => {
      timers.push(run);
      return timers.length;
    },
    cancel: () => undefined,
    random: () => 0,
  });
  const owner = {};
  const stop = subscribeDiagnostics({ log, scheduler, workOwner: owner });
  scheduler.update([
    {
      key: 'k',
      scope: 'srv',
      kind: 'catalog',
      interval: 30_000,
      initialDelay: 0,
      run: async () => {
        clock.tick(40);
      },
    },
  ]);
  scheduler.setEnabled(true);
  while (timers.length) timers.shift()!();
  for (let i = 0; i < 20; i++) await Promise.resolve();
  await scheduleProfileWork(owner, 'srv', async () => {
    clock.tick(5);
  });
  stop();
  const names = log
    .events()
    .map((event) => `${event.name}:${event.phase ?? ''}`);
  assert.deepEqual(names, [
    'job.catalog:started',
    'job.catalog:success',
    'queue:',
  ]);
  const [started, success, queue] = log.events();
  assert.equal(started.scope, 'srv');
  assert.equal(started.attrs?.trigger, 'periodic');
  assert.equal(success.ms, 40);
  assert.equal(success.outcome, 'ok');
  assert.equal(queue.scope, 'srv');
  assert.equal(queue.attrs?.priority, 'foreground');
  // Nothing listens after stop.
  let seen = 0;
  const off = observeProfileWork(owner, () => {
    seen++;
  });
  await scheduleProfileWork(owner, 'srv', async () => undefined);
  off();
  assert.equal(seen, 1);
  assert.equal(log.length, 3);
  scheduler.dispose();
});

const AT = Date.UTC(2026, 8, 19, 12, 0, 33, 104);
const sample: TimingEvent[] = [
  {
    at: AT,
    layer: 'renderer',
    name: 'job.catalog',
    scope: 'work',
    phase: 'started',
    attrs: { trigger: 'periodic', retry: 0, late: 12 },
  },
  {
    at: AT + 4,
    layer: 'renderer',
    name: 'invoke',
    scope: 'work',
    ms: 1146,
    outcome: 'ok',
    attrs: { command: 'list_profile_catalog' },
  },
  {
    at: AT + 6,
    layer: 'backend',
    name: 'agent.op',
    scope: 'work',
    ms: 612,
    outcome: 'ok',
    attrs: {
      op: 'ListKv',
      queue: 0,
      lock: 0,
      body: 608,
      auth: true,
      report: false,
    },
  },
  {
    at: AT + 7,
    layer: 'backend',
    name: 'agent.op',
    scope: 'work',
    ms: 540,
    outcome: 'ok',
    attrs: {
      op: 'ListKv',
      queue: 402,
      lock: 0,
      body: 136,
      auth: true,
      report: true,
      waited: 'security-root',
    },
  },
  {
    at: AT + 1_151,
    layer: 'renderer',
    name: 'catalog.project',
    scope: 'work',
    ms: 71,
    outcome: 'ok',
    attrs: { servers: 2, rosters: 0 },
  },
  {
    at: AT + 1_227,
    layer: 'renderer',
    name: 'job.catalog',
    scope: 'work',
    phase: 'success',
    ms: 1227,
    outcome: 'ok',
    attrs: { trigger: 'periodic', retry: 0 },
  },
  {
    at: AT + 8_000,
    layer: 'renderer',
    name: 'invoke',
    ms: 25_010,
    outcome: 'ok',
    attrs: { command: 'chat_request', action: 'poll-inbox' },
  },
  {
    at: AT + 8_001,
    layer: 'renderer',
    name: 'chat.poll',
    scope: 'acct#a91f',
    ms: 24_998,
    outcome: 'ok',
    attrs: { bumped: false },
  },
  {
    at: AT + 8_002,
    layer: 'renderer',
    name: 'chat.sync',
    scope: 'team#2c',
    ms: 388,
    outcome: 'ok',
    attrs: { changed: true, conversations: 14 },
  },
  {
    at: AT + 8_003,
    layer: 'renderer',
    name: 'chat.arrival',
    scope: 'team#2c',
    ms: 391,
    outcome: 'ok',
  },
  {
    at: AT + 8_004,
    layer: 'renderer',
    name: 'chat.send',
    scope: 'team#2c',
    id: 'm1',
    phase: 'prepared',
    ms: 620,
    outcome: 'ok',
  },
  {
    at: AT - 60 * 60_000,
    layer: 'renderer',
    name: 'invoke',
    ms: 5,
    outcome: 'ok',
    attrs: { command: 'too-old' },
  },
];

test('the formatter prints the timeline and the summaries as fixed text', () => {
  const text = formatTimings(sample, {
    now: AT + 10_000,
    header: ['agent ready · 2 servers · 6 teams · window visible'],
  });
  assert.equal(
    text,
    [
      '--- timing (last 15 min, 11 events, renderer 9 · backend 2 · agent-timed 2, times UTC) ---',
      'agent ready · 2 servers · 6 teams · window visible',
      '',
      '12:00:33.104  job.catalog        work                     started trigger=periodic retry=0 late=12ms',
      '12:00:33.108  invoke             work                     1.15s command=list_profile_catalog',
      '12:00:33.110    agent.op         work                     612ms op=ListKv queue=0ms lock=0ms body=608ms auth=true report=false',
      '12:00:33.111    agent.op         work                     540ms op=ListKv queue=402ms lock=0ms body=136ms auth=true report=true waited=security-root',
      '12:00:34.255  catalog.project    work                     71ms servers=2 rosters=0',
      '12:00:34.331  job.catalog        work                     success 1.23s trigger=periodic retry=0',
      '12:00:41.104  invoke                                      25.01s command=chat_request action=poll-inbox',
      '12:00:41.105  chat.poll          acct#a91f                25.00s bumped=false',
      '12:00:41.106  chat.sync          team#2c                  388ms changed=true conversations=14',
      '12:00:41.107  chat.arrival       team#2c                  391ms',
      '12:00:41.108  chat.send          team#2c · m1             prepared 620ms',
      '',
      '--- jobs ---',
      'kind     scope  runs  ok  fail  p50    p95    max    last',
      'catalog  work   1     1   0     1.23s  1.23s  1.23s  1.23s',
      '',
      '--- steps ---',
      'step             scope  n  ok  fail  p50   p95   max',
      'catalog.project  work   1  1   0     71ms  71ms  71ms',
      '',
      '--- commands (renderer → backend, poll-inbox excluded) ---',
      'command               n  ok  fail  p50    p95    max',
      'list_profile_catalog  1  1   0     1.15s  1.15s  1.15s',
      '',
      '--- agent ops (via backend, Chat/PollInbox excluded) ---',
      'op      n  fail  p50    p95    queue p50  lock p50  body p50  auth hit  report hit  waited behind',
      'ListKv  2  0     540ms  612ms  0ms        0ms       136ms     100%      50%         security-root ×1',
      '',
      '--- chat ---',
      'polls 1 (bumped 0, failed 0) · wait p50 25.00s',
      'syncs 1 (changed 1, failed 0) · p50 388ms p95 388ms',
      'arrivals 1 · bump→publish p50 391ms p95 391ms',
      'send steps prepared p50 620ms (1)',
    ].join('\n'),
  );
});

test('the formatter drops the oldest timeline lines to stay within its byte budget', () => {
  const events: TimingEvent[] = Array.from({ length: 200 }, (_, index) => ({
    at: AT + index,
    layer: 'renderer',
    name: 'invoke',
    ms: index,
    outcome: 'ok',
    attrs: { command: `command-${index}` },
  }));
  const text = formatTimings(events, { now: AT + 1_000, maxBytes: 6_000 });
  assert.ok(new TextEncoder().encode(text).length <= 6_000);
  assert.match(text, /^\(\d+ older lines omitted\)$/m);
  assert.match(text, /command-199/);
  assert.doesNotMatch(text, /^\d\d:\d\d:\d\d\.\d{3} {2}invoke .*command-0$/m);
  // Summaries are kept whole.
  assert.match(text, /--- commands/);
});

test('a request key hides the store ids it embeds behind their hashes', () => {
  const store =
    '{"accountAlias":"raymond","kind":"team","profile":"foks-app","teamAlias":"eng"}';
  assert.equal(redactKey('catalog:foks-app'), 'catalog:foks-app');
  assert.equal(
    redactKey(`catalog:foks-app:roster:${store}`),
    `catalog:foks-app:roster:store#${hashId(store)}`,
  );
  assert.equal(
    redactKey(JSON.stringify([store, 'sync-inbox'])),
    `store#${hashId(store)}:sync-inbox`,
  );
  assert.equal(
    redactKey(`team-discovery:${store}`),
    `team-discovery:store#${hashId(store)}`,
  );
  assert.equal(redactKey('x:{not json'), `x:#${hashId('{not json')}`);
  for (const key of [
    `catalog:foks-app:roster:${store}`,
    JSON.stringify([store, 'sync-inbox']),
  ])
    assert.doesNotMatch(redactKey(key), /raymond|eng/);
});

test('the connection probe and the popover read are counted but not listed', () => {
  const events: TimingEvent[] = [
    {
      at: AT,
      layer: 'renderer',
      name: 'invoke',
      ms: 1,
      outcome: 'ok',
      attrs: { command: 'take_agent_connection_loss' },
    },
    {
      at: AT + 1,
      layer: 'renderer',
      name: 'invoke',
      ms: 1,
      outcome: 'ok',
      attrs: { command: 'diagnostic_timings' },
    },
    {
      at: AT + 2,
      layer: 'renderer',
      name: 'invoke',
      ms: 40,
      outcome: 'ok',
      attrs: { command: 'list_stores' },
    },
  ];
  const text = formatTimings(events, { now: AT + 10 });
  assert.match(
    text,
    /^\(2 take_agent_connection_loss and diagnostic_timings calls counted below, not listed\)$/m,
  );
  assert.equal(text.match(/^\d\d:\d\d:\d\d\.\d{3} {2}invoke/gm)?.length, 1);
  assert.match(text, /^take_agent_connection_loss +1 /m);
  assert.match(text, /^diagnostic_timings +1 /m);
});
