import assert from 'node:assert/strict';
import test from 'node:test';
import { recoverPending } from '../src/chat/recover-pending';
import { enqueueProfileWork } from '../src/bridge';
import type { Bridge } from '../src/bridge';
import {
  coalesceStatus,
  RecoverySchedule,
} from '../src/chat/recovery-schedule';
import { cancelled, integrity, channelIntegrity } from '../src/chat/errors';
import type { TrackedOperation } from '../src/chat/operations';
import type { ChatAction } from '../src/chat-contract';

const operation = (id: string): TrackedOperation => ({
  id,
  channel: 'b'.repeat(32),
  kind: 'send-message',
  state: 'prepared',
  confirmation: null,
  rejection_code: null,
  statusUnknown: true,
});

for (const failure of [cancelled(), integrity()]) {
  test(`a ${failure.code} status result stops the entire recovery pass`, async () => {
    const calls: string[] = [];
    await assert.rejects(
      recoverPending(
        async (action) => {
          calls.push(action.action);
          if (action.action === 'status') throw failure;
        },
        () => [operation('old')],
        () => true,
      ),
    );
    assert.deepEqual(calls, ['pending', 'status']);
  });
}

test('an owner change cannot recover bodies from the replacement model', async () => {
  let current = true;
  let rows = [operation('old')];
  const calls: ChatAction[] = [];
  await assert.rejects(
    recoverPending(
      async (action) => {
        calls.push(action);
        if (action.action === 'status') {
          current = false;
          rows = [operation('new')];
        }
      },
      () => rows,
      () => current,
    ),
  );
  assert.deepEqual(
    calls.map((a) => a.action),
    ['pending', 'status'],
  );
});

test('recoverable status failures preserve separate bounded body recovery', async () => {
  const calls: ChatAction[] = [];
  const rows = Array.from({ length: 20 }, (_, i) => operation(String(i)));
  await recoverPending(
    async (action) => {
      calls.push(action);
      if (action.action === 'status')
        throw {
          code: 'offline',
          message: 'Offline',
          fatal: false,
          retryable: true,
          ambiguous: false,
        };
    },
    () => rows,
    () => true,
  );
  assert.equal(calls.filter((a) => a.action === 'status').length, 16);
  assert.equal(calls.filter((a) => a.action === 'operation-body').length, 16);
  assert.equal(calls.length, 33);
});

test('body selection uses state accepted after status recovery', async () => {
  let rows = [operation('observed')];
  const calls: string[] = [];
  await recoverPending(
    async (action) => {
      calls.push(action.action);
      if (action.action === 'status') rows = [{ ...rows[0], observed: true }];
    },
    () => rows,
    () => true,
  );
  assert.deepEqual(calls, ['pending', 'status']);
});

test('fair scheduling checks later IDs despite permanent failures in the first batch', async () => {
  let now = 0;
  const schedule = new RecoverySchedule(() => now);
  const rows = Array.from({ length: 40 }, (_, i) => ({
    ...operation(String(i)),
    state: 'uncertain' as const,
    statusUnknown: false,
  }));
  const status: string[] = [];
  const bodies: string[] = [];
  const request = async (action: ChatAction) => {
    if (action.action === 'status') status.push(action.operation);
    if (action.action === 'operation-body') bodies.push(action.operation);
    if (action.action !== 'pending') throw offline;
  };
  for (let i = 0; i < 3; i++)
    await recoverPending(
      request,
      () => rows,
      () => true,
      schedule,
    );
  assert.equal(new Set(status).size, 40);
  assert.equal(new Set(bodies).size, 40);
  assert.equal(status.length, 40);
  now = 1000;
  await recoverPending(
    request,
    () => rows,
    () => true,
    schedule,
  );
  assert.deepEqual(
    status.slice(40),
    rows.slice(0, 16).map((op) => op.id),
  );
});

test('successful but unresolved checks back off and removed or terminal rows are skipped', async () => {
  let now = 0;
  const schedule = new RecoverySchedule(() => now);
  let rows = [
    {
      ...operation('uncertain'),
      state: 'uncertain' as const,
      statusUnknown: false,
      text: 'retained',
    },
  ];
  let calls = 0;
  const request = async (action: ChatAction) => {
    if (action.action === 'status') calls++;
  };
  await recoverPending(
    request,
    () => rows,
    () => true,
    schedule,
  );
  await recoverPending(
    request,
    () => rows,
    () => true,
    schedule,
  );
  assert.equal(calls, 1);
  now = 2000;
  await recoverPending(
    request,
    () => rows,
    () => true,
    schedule,
  );
  assert.equal(calls, 2);
  rows = [];
  await recoverPending(
    request,
    () => rows,
    () => true,
    schedule,
  );
  const terminal = {
    ...operation('uncertain'),
    state: 'confirmed' as const,
    text: 'retained',
  };
  await recoverPending(
    request,
    () => [terminal],
    () => true,
    schedule,
  );
  assert.equal(calls, 2);
  // Removing the identity prunes its schedule; a new observation has no old delay.
  await recoverPending(
    request,
    () => [operation('uncertain')],
    () => true,
    schedule,
  );
  assert.equal(calls, 3);
});

test('channel quarantine stops that channel without starving another channel', async () => {
  const rows = [
    operation('blocked'),
    { ...operation('healthy'), channel: 'c'.repeat(32) },
  ];
  const calls: ChatAction[] = [];
  await recoverPending(
    async (action) => {
      calls.push(action);
      if (action.action === 'status' && action.operation === 'blocked')
        throw channelIntegrity();
    },
    () => rows,
    () => true,
  );
  assert.deepEqual(
    calls.filter((a) => a.action === 'operation-body').map((a) => a.operation),
    ['healthy'],
  );
});

test('selection is refreshed between checks and body cancellation stops later work', async () => {
  let rows = [operation('first'), operation('removed')];
  const calls: ChatAction[] = [];
  await assert.rejects(
    recoverPending(
      async (action) => {
        calls.push(action);
        if (action.action === 'status') rows = [rows[0]];
        if (action.action === 'operation-body') throw cancelled();
      },
      () => rows,
      () => true,
    ),
  );
  assert.deepEqual(
    calls.map((a) => a.action),
    ['pending', 'status', 'operation-body'],
  );
});

const offline = {
  code: 'offline',
  message: 'Offline',
  fatal: false,
  retryable: true,
  ambiguous: false,
};

test('explicit checks coalesce and take admission before the next automatic item', async () => {
  const bridge = {} as Bridge;
  const checks = new Map<string, Promise<void>>();
  const calls: string[] = [];
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let started!: () => void;
  const ready = new Promise<void>((resolve) => {
    started = resolve;
  });
  const request = (action: ChatAction) => {
    const send = () =>
      enqueueProfileWork(bridge, 'profile', async () => {
        if (action.action !== 'status') return;
        calls.push(action.operation);
        if (action.operation === 'first') {
          started();
          await gate;
        }
      });
    return action.action === 'status'
      ? coalesceStatus(checks, action.operation, send)
      : send();
  };
  const rows = [operation('first'), operation('second')].map((op) => ({
    ...op,
    text: 'retained',
  }));
  const background = recoverPending(
    request,
    () => rows,
    () => true,
  );
  await ready;
  const duplicate = request({ action: 'status', operation: 'first' });
  const explicit = request({ action: 'status', operation: 'manual' });
  release();
  await Promise.all([background, duplicate, explicit]);
  assert.deepEqual(calls, ['first', 'manual', 'second']);
  assert.equal(checks.size, 0);
});

test('new arrivals join behind existing eligible work', () => {
  const schedule = new RecoverySchedule(() => 5000);
  const first = operation('first');
  const old = operation('old');
  assert.equal(
    schedule.select('status', [first, old], new Set(), () => false)?.id,
    'first',
  );
  schedule.complete('status', 'first', true);
  assert.equal(
    schedule.select('status', [operation('new'), old], new Set(), () => false)
      ?.id,
    'old',
  );
});
