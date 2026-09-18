import assert from 'node:assert/strict';
import test from 'node:test';
import { enqueueProfileWork } from '../src/bridge';
import type { Bridge } from '../src/bridge';
import type { useWorkflowAccess } from '../src/workflow-context';
import { queuedDeviceWork, runDeviceMutation } from '../src/screens/devices/operation-controller';
import { refreshConnectedCards } from '../src/screens/devices/hardware-controller';
import { credentialCommand } from '../src/screens/devices/credential-workflow';
import { removeDevice } from '../src/screens/devices/revocation-workflow';
import { enrollmentSlot, provisionEnrollmentCommand, validEnrollmentAttempts } from '../src/screens/devices/enrollment-workflow';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

const unavailable = { code: 'unavailable', message: 'unavailable', retryable: true, fatal: false };

test('a stale device completion cannot refresh, close, or populate a different account', async () => {
  const write = deferred<number>();
  let current = true;
  const result = runDeviceMutation(
    () => write.promise,
    async () => assert.fail('stale completion reached sheet'),
    () => assert.fail('stale failure reached sheet'),
    () => assert.fail('stale refresh was attempted'),
    () => current,
  );
  current = false;
  write.resolve(1);
  assert.deepEqual(await result, {
    outcome: 'applied', synchronization: 'unobserved',
  });
});

test('device refresh failures remain applied even when onDone closes the sheet', async () => {
  let current = true;
  let pending = 0;
  const result = await runDeviceMutation(
    async () => 'written',
    async () => {
      current = false;
      throw unavailable;
    },
    () => assert.fail('write was reported as rejected'),
    () => { pending++; },
    () => current,
  );
  assert.equal(result.outcome, 'applied');
  assert.equal(pending, 1);
});

test('queued device work checks current workflow availability after acquiring its profile queue', async () => {
  const bridge = {} as Bridge;
  const held = deferred<void>();
  const first = enqueueProfileWork(bridge, 'profile', () => held.promise);
  let available = true;
  let checks = 0;
  let writes = 0;
  const access = {
    async run<T>(_operation: unknown, _target: unknown, task: () => Promise<T>) {
      checks++;
      if (!available) throw unavailable;
      return task();
    },
  } as ReturnType<typeof useWorkflowAccess>;
  const queued = queuedDeviceWork(bridge, 'profile', access, 'device-pair', { profile: 'profile' }, async () => {
    writes++;
  });
  assert.equal(checks, 0);
  available = false;
  const rejected = assert.rejects(queued, { code: 'unavailable' });
  held.resolve();
  await first;
  await rejected;
  assert.equal(checks, 1);
  assert.equal(writes, 0);
});

test('explicit card refresh discards a late result after the account changes', async () => {
  const cards = deferred<{ serial: number }[]>();
  const bridge = { listYubiCards: () => cards.promise } as unknown as Bridge;
  const access = {
    run: <T>(_operation: unknown, _target: unknown, task: () => Promise<T>) => task(),
  } as ReturnType<typeof useWorkflowAccess>;
  let current = true;
  const pending = refreshConnectedCards(bridge, access, 'p', () => current,
    () => assert.fail('stale cards displayed'),
    () => assert.fail('stale failure displayed'));
  current = false;
  cards.resolve([{ serial: 1 }]);
  await pending;
});

test('device removal re-reads native targets and checks access inside the queue before writing', async () => {
  const calls: string[] = [];
  const bridge = {
    listAccountDevices: async (id: string) => { calls.push(`list:${id}`); return []; },
    removeAccountDevice: async (id: string, deviceId: string) => {
      calls.push(`remove:${id}:${deviceId}`);
      return { deviceId };
    },
  } as unknown as Bridge;
  const access = {
    require: (operation: string) => { calls.push(operation); },
    run: async <T>(operation: string, _target: unknown, task: () => Promise<T>) => {
      calls.push(operation);
      return task();
    },
  } as ReturnType<typeof useWorkflowAccess>;
  const result = await removeDevice(bridge, access, {
    kind: 'account', id: 'acct:a', name: 'Account', server: 'p', account: 'a',
  }, 'device');
  assert.equal(result.deviceId, 'device');
  assert.deepEqual(calls, ['devices-list', 'list:acct:a', 'device-remove', 'remove:acct:a:device']);
});

test('credential requests preserve explicit resume and account-bound recovery targets', () => {
  assert.deepEqual(credentialCommand({
    action: 'resume-rotation', profile: 'p', alias: 'key', pin: '', other: '', confirmation: '',
  }), {
    command: 'resume_yubi_management_key', args: { profile: 'p', alias: 'key' },
  });
  assert.equal(credentialCommand({
    action: 'recover-management', profile: 'p', alias: 'key', pin: '', other: '', confirmation: '',
  }), undefined);
  assert.deepEqual(provisionEnrollmentCommand({
    accountStoreId: 'acct:a', targetAlias: ' new ', deviceName: ' Card ', cardSerial: 2, pin: 'pin', puk: 'puk',
  }), {
    command: 'provision_yubi_device',
    args: {
      accountStoreId: 'acct:a', targetAlias: 'new', deviceName: 'Card', cardSerial: 2,
      pin: 'pin', puk: 'puk', signingSlot: 0x82, pqSlot: 0x83, pinAttempts: 3, pukAttempts: 3,
    },
  });
  assert.equal(enrollmentSlot('0x82'), 0x82);
  assert.equal(enrollmentSlot('82'), null);
  assert.equal(validEnrollmentAttempts(3, 255), true);
  assert.equal(validEnrollmentAttempts(0, 256), false);
});
