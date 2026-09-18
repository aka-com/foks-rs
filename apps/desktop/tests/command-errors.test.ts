import assert from 'node:assert/strict';
import test from 'node:test';
import {
  commandRecovery,
  normalizeCommandError,
  normalizeMutationError,
  onAgentReadinessRequired,
  type CommandError,
} from '../src/bridge/errors';
import { checked, checkedMutation } from '../src/bridge/transport';
import { tauriBridge } from '../src/bridge/tauri';
import { loadSnapshot } from '../src/bridge/snapshot';
import { failureOutcome, synchronizeApplied } from '../src/operation-outcome';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';

const error = (code: string, fatal = false): CommandError => ({
  code,
  message: code,
  retryable: false,
  ambiguous: false,
  fatal,
});

function native(
  t: { after(fn: () => void): void },
  invoke: () => Promise<unknown>,
) {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'window');
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: { __TAURI_INTERNALS__: { invoke } },
  });
  t.after(() => {
    if (previous) Object.defineProperty(globalThis, 'window', previous);
    else delete (globalThis as { window?: Window }).window;
  });
}

test('recovery follows explicit cause and scope rather than fatality alone', () => {
  for (const [code, kind, scope] of [
    ['cancelled', 'ignore', 'request'],
    ['catalog-read-retired', 'ignore', 'request'],
    ['agent-lost', 'reconnect', 'agent'],
    ['bootstrap-required', 'reconnect', 'agent'],
    ['protocol', 'quarantine', 'agent'],
    ['response-binding', 'quarantine', 'agent'],
    ['chat-integrity', 'quarantine', 'account'],
    ['chat-channel-integrity', 'quarantine', 'channel'],
    ['unsupported-schema', 'quarantine', 'profile'],
    ['chat-access-denied', 'revalidate', 'team'],
    ['unknown', 'refresh', 'request'],
  ])
    assert.deepEqual(commandRecovery(error(code, true)), { kind, scope });
});

test('unknown read failures stay local while unknown mutation outcomes remain conservative', async () => {
  const cause = new Error('Projection failed.');
  assert.equal(normalizeCommandError(cause).ambiguous, false);
  assert.equal(normalizeCommandError(cause).fatal, false);
  assert.equal(normalizeMutationError(cause).ambiguous, true);
  assert.equal(failureOutcome(cause), 'unknown');
  const result = await synchronizeApplied(async () => {
    throw cause;
  });
  assert.equal(result.outcome, 'applied');
  assert.equal(result.synchronization, 'pending');
  if (result.synchronization === 'pending')
    assert.equal(result.error.ambiguous, false);
  await assert.rejects(
    loadSnapshot(mockBridge(FIXTURE), FIXTURE, 1, undefined, () => false),
    {
      code: 'catalog-read-retired',
      fatal: false,
      ambiguous: false,
    },
  );
});

test('malformed invocation errors do not imply agent loss and mutation wrappers retain uncertainty', async (t) => {
  native(t, async () => {
    throw 'native rejection';
  });
  const events: string[] = [];
  t.after(onAgentReadinessRequired((e) => events.push(e.code)));
  await assert.rejects(
    checked('read', undefined, (value) => value),
    {
      code: 'invalid-command-error',
      origin: 'invoke',
      fatal: false,
      ambiguous: false,
    },
  );
  await assert.rejects(
    checkedMutation('write', undefined, (value) => value),
    {
      code: 'invalid-command-error',
      origin: 'invoke',
      fatal: false,
      ambiguous: true,
    },
  );
  await assert.rejects(tauriBridge.listPendingOperations('profile'), {
    ambiguous: false,
  });
  await assert.rejects(
    tauriBridge.createTextItem({
      storeId: 'store',
      path: '/test',
      value: 'value',
    }),
    { ambiguous: true },
  );
  await assert.rejects(tauriBridge.chat('team', { action: 'sync-inbox' }), {
    ambiguous: false,
  });
  await assert.rejects(
    tauriBridge.chat('team', {
      action: 'prepare-channel',
      submission: 'submission',
      name: 'name',
      description: '',
      admin: false,
    }),
    { ambiguous: true },
  );
  assert.deepEqual(events, []);
});

test('response validation is distinct from invocation failure and never publishes invalid data', async (t) => {
  native(t, async () => ({}));
  const decode = () => {
    throw new Error('Invalid response.');
  };
  await assert.rejects(checked('read', undefined, decode), {
    code: 'invalid-response',
    origin: 'response',
    fatal: false,
    ambiguous: false,
  });
  await assert.rejects(checkedMutation('write', undefined, decode), {
    code: 'invalid-response',
    origin: 'response',
    fatal: false,
    ambiguous: true,
    retryable: false,
  });
  const events: string[] = [];
  t.after(onAgentReadinessRequired((e) => events.push(e.code)));
  await assert.rejects(
    checked('read', undefined, () => {
      throw error('response-binding', true);
    }),
    {
      code: 'response-binding',
      fatal: true,
      ambiguous: false,
    },
  );
  assert.deepEqual(events, ['response-binding']);
});

test('typed admission refusals and ambiguous outcomes survive the mutation boundary', async (t) => {
  let cause = error('conflict');
  native(t, async () => {
    throw cause;
  });
  await assert.rejects(
    checkedMutation('write', undefined, (value) => value),
    { code: 'conflict', ambiguous: false },
  );
  cause = { ...error('deadline-exceeded'), retryable: true, ambiguous: true };
  await assert.rejects(
    checked('read', undefined, (value) => value),
    { code: 'deadline-exceeded', ambiguous: false },
  );
  await assert.rejects(
    checkedMutation('write', undefined, (value) => value),
    { code: 'deadline-exceeded', ambiguous: true },
  );
});
