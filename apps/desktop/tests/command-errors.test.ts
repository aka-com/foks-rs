import assert from 'node:assert/strict';
import test from 'node:test';
import {
  commandRecovery,
  normalizeCommandError,
  normalizeMutationError,
  onAgentReadinessRequired,
  type CommandError,
} from '../src/bridge/errors';
import { createBridge } from '../src/bridge/create';
import {
  createTransport,
  tauriTransport,
  type TransportHost,
} from '../src/bridge/transport';
import { tauriBridge } from '../src/bridge/tauri';
import { loadSnapshot } from '../src/bridge/snapshot';
import {
  failureOutcome,
  synchronizeApplied,
} from '../src/data/operation-outcome';
import { FIXTURE } from '../src/mock/fixture';
import { mockBridge } from '../src/mock/mock-bridge';

const { checked, checkedMutation } = tauriTransport;

/** The chat view a test names when it calls the bridge directly. */
const VIEW = 'cd'.repeat(16);
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
    ['deadline-domain-mismatch', 'quarantine', 'agent'],
    ['upload-spool-full', 'refresh', 'request'],
    ['upload-clock-untrusted', 'refresh', 'request'],
    ['deadline-clock', 'refresh', 'request'],
    ['deadline-clock-unavailable', 'refresh', 'request'],
    ['missing-deadline', 'refresh', 'request'],
    ['app-lock-unavailable', 'refresh', 'request'],
    ['authentication-interrupted', 'retry', 'request'],
    ['unknown', 'refresh', 'request'],
  ])
    assert.deepEqual(commandRecovery(error(code, true)), { kind, scope });
});

test('deadline, lock-state and upload admission failures have explicit outcomes', () => {
  for (const [code, outcome] of [
    ['deadline-clock', 'unknown'],
    ['deadline-clock-unavailable', 'unknown'],
    ['deadline-domain-mismatch', 'unknown'],
    ['missing-deadline', 'unknown'],
    ['app-lock-unavailable', 'unknown'],
    ['missing-deadline-policy', 'not-started'],
    ['upload-expired', 'rejected'],
    ['upload-identity-conflict', 'rejected'],
    ['upload-generation-mismatch', 'rejected'],
    ['upload-legacy-read-only', 'rejected'],
    ['upload-pending-full', 'rejected'],
    ['upload-retention-full', 'rejected'],
    ['upload-spool-full', 'rejected'],
    ['upload-clock-untrusted', 'rejected'],
    ['upload-future', 'rejected'],
    ['authentication-interrupted', 'rejected'],
  ])
    assert.equal(failureOutcome(error(code)), outcome, code);
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
  await assert.rejects(
    tauriBridge.chat('team', { action: 'sync-inbox' }, VIEW),
    {
      ambiguous: false,
    },
  );
  await assert.rejects(
    tauriBridge.chat(
      'team',
      {
        action: 'prepare-channel',
        submission: 'submission',
        name: 'name',
        description: '',
        admin: false,
      },
      VIEW,
    ),
    { ambiguous: true },
  );
  assert.deepEqual(events, []);
});

test('a bridge over an injected host decodes its responses and classifies reads and mutations', async () => {
  const calls: string[] = [];
  let respond: () => unknown = () => {
    throw 'host rejection';
  };
  let hover: ((payload: unknown) => void) | undefined;
  const host: TransportHost = {
    invoke: async (command) => {
      calls.push(command);
      return respond();
    },
    listen: async (event, handler) => {
      if (event === 'foks://drop-hover') hover = handler;
      return () => undefined;
    },
    channel: () => ({ onmessage: () => undefined }),
  };
  const bridge = createBridge(createTransport(host), { native: false });
  assert.equal(bridge.native, false);
  const card = { profile: 'personal', alias: 'card' };
  await assert.rejects(
    bridge.runYubi({ command: 'yubi_pin_status', args: card }),
    { code: 'invalid-command-error', origin: 'invoke', ambiguous: false },
  );
  await assert.rejects(
    bridge.runYubi({
      command: 'change_yubi_pin',
      args: { ...card, oldPin: '123456', newPin: '654321' },
    }),
    { code: 'invalid-command-error', origin: 'invoke', ambiguous: true },
  );
  await assert.rejects(
    bridge.invitation('personal', 'satoshi', { action: 'list' }, null),
    { ambiguous: false },
  );
  await assert.rejects(
    bridge.invitation(
      'personal',
      'satoshi',
      { action: 'accept', invite: 'invite' },
      null,
    ),
    { ambiguous: true },
  );
  respond = () => ({});
  await assert.rejects(
    bridge.readItem({
      storeId: 'store',
      path: '/a',
      version: 1,
      direntId: 'aa'.repeat(16),
    }),
    { code: 'invalid-response', origin: 'response', ambiguous: false },
  );
  await assert.rejects(bridge.createFolder({ storeId: 'store', path: '/a' }), {
    code: 'invalid-response',
    origin: 'response',
    ambiguous: true,
  });
  assert.deepEqual(calls, [
    'yubi_pin_status',
    'change_yubi_pin',
    'invitation_request',
    'invitation_request',
    'read_item',
    'create_folder',
  ]);
  const hovering: boolean[] = [];
  await bridge.onDropHover((event) => hovering.push(event.hovering));
  hover?.({ hovering: true });
  assert.throws(() => hover?.({ hovering: 'yes' }));
  assert.deepEqual(hovering, [true]);
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
