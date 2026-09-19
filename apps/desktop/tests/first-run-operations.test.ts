import { retainSetup, retainedSetups } from '../src/first-run-recovery';
import assert from 'node:assert/strict';
import test from 'node:test';
import { installDom } from './lib/dom-harness';
import {
  executeProvisioning,
  provisioningInFlight,
} from '../src/first-run-operations';
import {
  initialFirstRun,
  decodeFirstRunCheckpoint,
  FIRST_RUN_CHECKPOINT_KEY,
  encodeFirstRunCheckpoint,
  type FirstRunCheckpoint,
} from '../src/first-run-state';
import type { Bridge } from '../src/bridge';

installDom({ url: 'http://localhost/' });
const saved: FirstRunCheckpoint = {
  ...initialFirstRun('own', 'operation-pending'),
  profile: {
    profile: 'personal',
    hostId: `02${'2'.repeat(64)}`,
    lookupName: 'localhost',
    canonicalName: 'localhost',
    acceptance: 'unchanged',
    chain: 1,
    epoch: 1,
  },
  serverAddress: 'localhost:4430',
  provisioning: {
    id: 'attempt-1',
    kind: 'signup',
    alias: 'personal',
    deviceName: 'Mac',
    back: 'account',
  },
};
const read = () =>
  decodeFirstRunCheckpoint(
    window.localStorage.getItem(FIRST_RUN_CHECKPOINT_KEY),
  );

test('v3 intent round trips without secret fields; v2 checkpoints remain readable', () => {
  assert.deepEqual(readIntent(saved)?.provisioning, saved.provisioning);
  assert.equal(
    readIntent({
      ...saved,
      provisioning: { ...saved.provisioning!, phrase: 'secret' },
    } as FirstRunCheckpoint),
    null,
  );
  const old = { ...initialFirstRun('own'), version: 2 };
  assert.equal(decodeFirstRunCheckpoint(JSON.stringify(old))?.version, 3);
});
function readIntent(value: FirstRunCheckpoint) {
  return decodeFirstRunCheckpoint(encodeFirstRunCheckpoint(value));
}

test('acknowledgment is durably recorded after the initiating screen has gone away', async () => {
  const bridge = {} as Bridge;
  let complete!: () => void;
  const result = executeProvisioning(
    bridge,
    saved,
    () =>
      new Promise<void>((resolve) => {
        complete = resolve;
      }),
  );
  assert.equal(read()?.provisioning?.id, 'attempt-1');
  assert.ok(provisioningInFlight(bridge, 'attempt-1'));
  await assert.rejects(
    executeProvisioning(bridge, saved, async () => {}),
    /still running/,
  );
  complete();
  await result;
  assert.equal(read()?.state, 'identity-pending');
  assert.equal(read()?.provisionedAccount?.alias, 'personal');
});

test('ambiguous outcome and unsuccessful resume retain the exact intent', async () => {
  const bridge = {} as Bridge;
  await executeProvisioning(bridge, saved, async () => {
    throw { code: 'ambiguous', ambiguous: true, message: 'Lost reply' };
  });
  assert.equal(read()?.state, 'operation-pending');
  await executeProvisioning(
    bridge,
    saved,
    async () => {
      throw { code: 'pending-operation-not-found', message: 'Not found' };
    },
    true,
  );
  assert.equal(read()?.provisioning?.id, saved.provisioning?.id);
});

test('definitive initial rejection permits correction without discarding the selected server', async () => {
  await executeProvisioning({} as Bridge, saved, async () => {
    throw {
      code: 'invalid-request',
      message: 'Invalid username',
      ambiguous: false,
      retryable: false,
      fatal: false,
    };
  });
  assert.equal(read()?.state, 'account');
  assert.equal(read()?.profile?.hostId, saved.profile?.hostId);
  assert.equal(read()?.provisioning, undefined);
});

test('unclassified failures cannot authorize another initial mutation', async () => {
  await executeProvisioning({} as Bridge, saved, async () => {
    throw new Error('Lost response');
  });
  assert.equal(read()?.provisioning?.id, saved.provisioning?.id);
  assert.equal(read()?.state, 'operation-pending');
});

test('restarting the wizard preserves a late acknowledgment without restoring the old wizard', async () => {
  window.localStorage.clear();
  let complete!: () => void;
  const pending = executeProvisioning(
    {} as Bridge,
    saved,
    () =>
      new Promise<void>((resolve) => {
        complete = resolve;
      }),
  );
  retainSetup(saved);
  window.localStorage.setItem(
    FIRST_RUN_CHECKPOINT_KEY,
    encodeFirstRunCheckpoint(initialFirstRun()),
  );
  complete();
  await pending;
  assert.equal(read()?.state, 'who');
  assert.equal(retainedSetups()[0].checkpoint.state, 'identity-pending');
  assert.equal(
    retainedSetups()[0].checkpoint.provisionedAccount?.alias,
    'personal',
  );
});

test('retained attempts reject malformed secret-bearing intents', () => {
  window.localStorage.clear();
  assert.throws(() =>
    retainSetup({
      ...saved,
      provisioning: { ...saved.provisioning!, phrase: 'SECRET' },
    } as FirstRunCheckpoint),
  );
  assert.deepEqual(retainedSetups(), []);
});
