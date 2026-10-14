import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChatClock } from '../src/chat/clock';
import type { chatClient } from '../src/chat/client';
import { newOutgoing } from '../src/chat/send/outgoing';
import {
  ACTIVE_TICK,
  IDLE_TICK,
  SendScheduler,
  nextTickDelay,
  teamHasWork,
} from '../src/chat/send/scheduler';
import { newTeam, type Team } from '../src/chat/send/team';

const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

class Clock implements ChatClock {
  time = 1_000_000;
  next = 0;
  timers = new Map<number, { due: number; fn: () => void }>();
  now = () => this.time;
  random = () => 0.5;
  later = (fn: () => void, delay: number) => {
    const id = ++this.next;
    this.timers.set(id, { due: this.time + delay, fn });
    return id;
  };
  cancel = (id: unknown) => {
    this.timers.delete(id as number);
  };
  /** The delay to the one armed timer, or undefined when none is armed. */
  armed(): number | undefined {
    const timers = [...this.timers.values()];
    assert.ok(timers.length <= 1);
    return timers[0] && timers[0].due - this.time;
  }
  async advance(milliseconds: number) {
    const end = this.time + milliseconds;
    for (;;) {
      const next = [...this.timers]
        .filter(([, timer]) => timer.due <= end)
        .sort((a, b) => a[1].due - b[1].due)[0];
      if (!next) break;
      this.time = next[1].due;
      this.timers.delete(next[0]);
      next[1].fn();
      await settle();
    }
    this.time = end;
    await settle();
  }
}

function team(storeId: string, profile = storeId): Team {
  const t = newTeam({
    storeId,
    profile,
    scope: null,
    client: { dispose() {} } as unknown as ReturnType<typeof chatClient>,
  });
  t.recovered = true;
  return t;
}

test('a recovered team with nothing in hand has no work unless a channel is unopened', () => {
  const t = team('team:a');
  assert.equal(
    teamHasWork(t, () => false),
    false,
  );
  assert.equal(
    teamHasWork(t, () => true),
    true,
  );
  t.recovered = false;
  assert.equal(
    teamHasWork(t, () => false),
    true,
  );
  t.recovered = true;
  t.operations.push({
    id: 'a'.repeat(32),
    channel: 'b'.repeat(32),
    kind: 'send-message',
    state: 'confirmed',
    confirmation: { kind: 'message-sent', sequence: '1' },
    rejection_code: null,
  });
  // A settled operation is not work.
  assert.equal(
    teamHasWork(t, () => false),
    false,
  );
  t.operations[0] = { ...t.operations[0], statusUnknown: true };
  assert.equal(
    teamHasWork(t, () => false),
    true,
  );
  t.operations.length = 0;
  t.messages.push(
    newOutgoing({
      channel: 'b'.repeat(32),
      submission: 's'.repeat(32),
      text: 'hello',
      createdAt: 0,
      delivery: { stage: 'queued' },
    }),
  );
  assert.equal(
    teamHasWork(t, () => false),
    true,
  );
});

test('the tick waits for the soonest gate of a team with work, within its bounds', () => {
  const now = 10_000;
  const a = team('team:a');
  const b = team('team:b');
  assert.equal(
    nextTickDelay([a, b], now, () => false),
    IDLE_TICK,
  );
  a.due = now - 5;
  assert.equal(
    nextTickDelay([a, b], now, (t) => t === a),
    ACTIVE_TICK,
  );
  a.due = now + 4_000;
  b.due = now + 3_000;
  assert.equal(
    nextTickDelay([a, b], now, () => true),
    3_000,
  );
  a.due = now + 60_000;
  assert.equal(
    nextTickDelay([a], now, () => true),
    IDLE_TICK,
  );
});

function scheduler(
  clock: Clock,
  teams: Team[],
  run: (team: Team) => Promise<void>,
) {
  const state = { active: true, synchronized: 0, runs: [] as string[] };
  const s = new SendScheduler(clock, {
    active: () => state.active,
    teams: () => teams,
    synchronize: () => {
      state.synchronized++;
    },
    outstanding: () => true,
    current: () => true,
    run: (t) => {
      state.runs.push(t.storeId);
      return run(t);
    },
  });
  return Object.assign(state, { scheduler: s });
}

test('a tick starts one pass per profile and closes each team gate', async () => {
  const clock = new Clock();
  const teams = [
    team('team:a', 'acme'),
    team('team:b', 'acme'),
    team('team:c', 'other'),
  ];
  const passes: (() => void)[] = [];
  const h = scheduler(
    clock,
    teams,
    () =>
      new Promise<void>((resolve) => {
        passes.push(resolve);
      }),
  );
  h.scheduler.arm(true);
  assert.equal(clock.armed(), ACTIVE_TICK);
  await clock.advance(ACTIVE_TICK);
  assert.equal(h.synchronized, 1);
  // team:b shares a profile with team:a, whose pass is still running.
  assert.deepEqual(h.runs, ['team:a', 'team:c']);
  assert.equal(teams[0].due, clock.time + 2000);
  assert.equal(teams[1].due, 0);
  for (const pass of passes) pass();
  await settle();
  await clock.advance(ACTIVE_TICK);
  assert.deepEqual(h.runs, ['team:a', 'team:c', 'team:b']);
});

test('a failed pass closes the team gate for longer', async () => {
  const clock = new Clock();
  const t = team('team:a');
  const h = scheduler(clock, [t], () =>
    Promise.reject(new Error('unavailable')),
  );
  h.scheduler.arm(true);
  await clock.advance(ACTIVE_TICK);
  assert.deepEqual(h.runs, ['team:a']);
  assert.equal(t.due, clock.time + 5000);
});

test('an inactive scheduler arms nothing and its tick does nothing', async () => {
  const clock = new Clock();
  const h = scheduler(clock, [team('team:a')], () => Promise.resolve());
  h.scheduler.arm(true);
  h.active = false;
  h.scheduler.arm();
  assert.equal(clock.armed(), undefined);
  h.active = true;
  h.scheduler.arm(true);
  h.active = false;
  await clock.advance(ACTIVE_TICK);
  assert.equal(h.synchronized, 0);
  assert.deepEqual(h.runs, []);
});
