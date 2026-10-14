import assert from 'node:assert/strict';
import test from 'node:test';
import type {
  ChatAction,
  ChatOperation,
  ChatReply,
} from '../src/chat-contract';
import { REQUEST_RETRY, retryDelay } from '../src/chat/backoff';
import type { chatClient } from '../src/chat/client';
import { newOutgoing, transition } from '../src/chat/send/outgoing';
import {
  nextRecoveryJob,
  planRecoveryJobs,
  pruneChecks,
  recover,
  runChecked,
  type RecoveryHost,
} from '../src/chat/send/recovery';
import { newTeam, retire, type Team } from '../src/chat/send/team';

const CHANNEL = 'b'.repeat(32);
const op = (
  id: string,
  state: ChatOperation['state'],
  kind: ChatOperation['kind'] = 'send-message',
): ChatOperation => ({
  id: id.repeat(32),
  channel: CHANNEL,
  kind,
  state,
  confirmation: null,
  rejection_code: null,
});
const team = (): Team =>
  newTeam({
    storeId: 'team:eng',
    profile: 'acme',
    scope: null,
    client: { dispose() {} } as unknown as ReturnType<typeof chatClient>,
  });
const keys = (jobs: { key: string }[]) => jobs.map((job) => job.key).sort();

test('an operation of unknown outcome is reconciled where its channel can be read, else read back', () => {
  const t = team();
  const uncertain = op('a', 'uncertain');
  const unknown = {
    ...op('c', 'prepared', 'create-channel'),
    statusUnknown: true,
  };
  t.operations.push(uncertain, unknown, op('d', 'confirmed', 'create-channel'));
  assert.deepEqual(
    planRecoveryJobs(t, new Map(), () => true).map((job) =>
      job.kind === 'status' ? [job.operation, job.action] : job.kind,
    ),
    [
      [uncertain.id, 'reconcile'],
      // An unverified sent message whose text no row holds is read back too.
      'body',
      [unknown.id, 'reconcile'],
    ],
  );
  assert.deepEqual(
    planRecoveryJobs(t, new Map(), () => false).map((job) =>
      job.kind === 'status' ? job.action : job.kind,
    ),
    ['status', 'status'],
  );
});

test('an operation whose message is being driven is left to the drive', () => {
  const t = team();
  const uncertain = op('a', 'uncertain');
  t.operations.push(uncertain);
  const message = transition(
    newOutgoing({
      channel: CHANNEL,
      submission: 's'.repeat(32),
      text: 'hello',
      createdAt: 0,
      delivery: { stage: 'uncertain', op: uncertain },
    }),
    { type: 'started', step: 'check' },
  );
  t.messages.push(message);
  assert.deepEqual(
    planRecoveryJobs(t, new Map(), () => true),
    [],
  );
});

test('cleanup reads back a rejected message no row holds before it finalizes', () => {
  const t = team();
  const rejected = op('a', 'rejected');
  const confirmed = op('c', 'confirmed');
  const obligations = new Map([
    [rejected.id, rejected],
    [confirmed.id, confirmed],
  ]);
  const plan = (deliverable: boolean) =>
    planRecoveryJobs(t, obligations, () => deliverable).map((job) =>
      job.kind === 'cleanup' ? [job.operation.id, job.body] : job.kind,
    );
  assert.deepEqual(plan(true), [
    [rejected.id, true],
    [confirmed.id, false],
  ]);
  assert.deepEqual(plan(false), [
    [rejected.id, false],
    [confirmed.id, false],
  ]);
});

test('saved and access-paused messages are driven only when sent automatically', () => {
  const t = team();
  const saved = newOutgoing({
    channel: CHANNEL,
    submission: 's'.repeat(32),
    text: 'hello',
    createdAt: 0,
    delivery: { stage: 'preparing', ambiguous: true },
    intent: 'pending',
  });
  const prepared = op('a', 'prepared');
  const cleared = newOutgoing({
    channel: CHANNEL,
    submission: 't'.repeat(32),
    text: 'world',
    createdAt: 0,
    delivery: { stage: 'prepared', op: prepared },
    intent: 'pending',
  });
  const rebuilt = newOutgoing({
    id: 'r'.repeat(32),
    channel: CHANNEL,
    submission: '',
    text: 'body',
    createdAt: 0,
    automatic: false,
    intent: 'pending',
    delivery: { stage: 'preparing', ambiguous: true },
  });
  t.messages.push(saved, cleared, rebuilt);
  assert.deepEqual(keys(planRecoveryJobs(t, new Map(), () => true)), [
    `intent:${'t'.repeat(32)}`,
    `send:${'s'.repeat(32)}`,
  ]);
  // Without access, only the saved copy's cleanup can proceed.
  assert.deepEqual(keys(planRecoveryJobs(t, new Map(), () => false)), [
    `intent:${'t'.repeat(32)}`,
  ]);
});

test('the next job is the longest-due one not yet run in this recovery', () => {
  const jobs = [
    { key: 'status:a', kind: 'body' as const, operation: 'a', channel: '' },
    { key: 'status:b', kind: 'body' as const, operation: 'b', channel: '' },
    { key: 'status:c', kind: 'body' as const, operation: 'c', channel: '' },
  ];
  const checks: Team['checks'] = new Map([
    ['status:a', { due: 50, failures: 1 }],
    ['status:b', { due: 200, failures: 1 }],
  ]);
  assert.equal(nextRecoveryJob(jobs, checks, new Set(), 100)?.key, 'status:c');
  assert.equal(
    nextRecoveryJob(jobs, checks, new Set(['status:c']), 100)?.key,
    'status:a',
  );
  assert.equal(
    nextRecoveryJob(jobs, checks, new Set(['status:a', 'status:c']), 100),
    undefined,
  );
});

test('a checked job backs off on failure and returns to the base interval on success', async () => {
  const checks: Team['checks'] = new Map();
  let now = 1000;
  let runs = 0;
  const fail = () => {
    runs++;
    return Promise.reject(new Error('unavailable'));
  };
  await runChecked(checks, 'k', () => now, fail);
  assert.deepEqual(checks.get('k'), {
    failures: 1,
    due: now + retryDelay(1, REQUEST_RETRY),
  });
  // Not due yet: nothing runs and the record stands.
  await runChecked(checks, 'k', () => now, fail);
  assert.equal(runs, 1);
  now = checks.get('k')!.due;
  await runChecked(checks, 'k', () => now, fail);
  assert.equal(checks.get('k')!.failures, 2);
  now = checks.get('k')!.due;
  await runChecked(
    checks,
    'k',
    () => now,
    () => Promise.resolve(),
  );
  assert.deepEqual(checks.get('k'), {
    failures: 0,
    due: now + REQUEST_RETRY.base,
  });
});

test('backoff records are dropped with their operation or message', () => {
  const t = team();
  t.operations.push(op('a', 'uncertain'));
  t.checks.set(`status:${'a'.repeat(32)}`, { due: 1, failures: 1 });
  t.checks.set(`status:${'z'.repeat(32)}`, { due: 1, failures: 1 });
  pruneChecks(t);
  assert.deepEqual([...t.checks.keys()], [`status:${'a'.repeat(32)}`]);
});

function host(
  t: Team,
  reply: (action: ChatAction) => ChatReply['result'],
): RecoveryHost & { actions: string[]; published: number } {
  const state = { actions: [] as string[], published: 0 };
  return Object.assign(state, {
    current: (epoch: number) => t.epoch === epoch,
    deliverable: () => true,
    now: () => 0,
    request: async (action: ChatAction) => {
      state.actions.push(action.action);
      return { scope: null, result: reply(action) } as unknown as ChatReply;
    },
    drain: () => {},
    drive: () => Promise.resolve(),
    clearIntent: () => Promise.resolve(),
    publish: () => {
      state.published++;
    },
  });
}

test('a recovery reads the pending lists, finalizes what cleanup holds, and records that it ran', async () => {
  const t = team();
  const confirmed = op('a', 'confirmed');
  const h = host(t, (action) => {
    if (action.action === 'cleanup-pending')
      return { kind: 'cleanup-pending', operations: [confirmed] };
    if (action.action === 'pending') return { kind: 'pending', operations: [] };
    return { kind: 'operation', operation: confirmed };
  });
  await recover(t, h);
  assert.deepEqual(h.actions, ['pending', 'cleanup-pending', 'finalize']);
  assert.equal(t.recovered, true);
  assert.equal(t.jobs, false);
  assert.equal(h.published, 1);
});

test('a failed finalize is reported as the team cleanup error and left for a later recovery', async () => {
  const t = team();
  const confirmed = op('a', 'confirmed');
  // The service tracks what cleanup reports; observed, its body is not read.
  t.operations.push({ ...confirmed, observed: true });
  const h = host(t, (action) => {
    if (action.action === 'cleanup-pending')
      return { kind: 'cleanup-pending', operations: [confirmed] };
    if (action.action === 'pending') return { kind: 'pending', operations: [] };
    throw {
      code: 'unavailable',
      message: 'Finalize failed',
      fatal: false,
      ambiguous: false,
      retryable: true,
    };
  });
  await recover(t, h);
  assert.equal(t.cleanupError, 'Finalize failed');
  assert.equal(t.jobs, true);
  assert.equal(t.checks.get(`cleanup:${confirmed.id}`)?.failures, 1);
  assert.deepEqual(h.actions, ['pending', 'cleanup-pending', 'finalize']);
});

test('a finalize that fails after the team was retired leaves no cleanup error', async () => {
  const t = team();
  const confirmed = op('a', 'confirmed');
  t.operations.push({ ...confirmed, observed: true });
  const h = host(t, (action) => {
    if (action.action === 'cleanup-pending')
      return { kind: 'cleanup-pending', operations: [confirmed] };
    if (action.action === 'pending') return { kind: 'pending', operations: [] };
    // Retiring disposes the client, which cancels the request in flight.
    retire(t);
    throw new Error('Conversation closed.');
  });
  await recover(t, h);
  assert.deepEqual(h.actions, ['pending', 'cleanup-pending', 'finalize']);
  assert.equal(t.cleanupError, '');
});

test('a team retired during recovery is not marked recovered', async () => {
  const t = team();
  const h = host(t, (action) => {
    if (action.action === 'pending') {
      t.epoch++;
      return { kind: 'pending', operations: [] };
    }
    return { kind: 'cleanup-pending', operations: [] };
  });
  await recover(t, h);
  assert.deepEqual(h.actions, ['pending']);
  assert.equal(t.recovered, false);
  assert.equal(h.published, 0);
});
