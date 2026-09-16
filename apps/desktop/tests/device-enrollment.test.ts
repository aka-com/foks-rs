import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeYubiAccounts } from '../src/bridge';
import {
  enrollmentForCard,
  enrollmentForDevice,
} from '../src/screens/device-model';
const id = '0802' + 'ab'.repeat(32);

test('enrollment identity survives the bridge and selects the exact device and card', () => {
  const entries = decodeYubiAccounts([
    {
      alias: 'other',
      state: 'complete',
      deviceId: '0803' + 'cd'.repeat(32),
      cardSerial: 222,
    },
    { alias: 'work', state: 'complete', deviceId: id, cardSerial: 111 },
  ]);
  assert.equal(enrollmentForDevice(entries, id)?.alias, 'work');
  assert.equal(enrollmentForCard(entries, 111)?.alias, 'work');
  assert.equal(enrollmentForCard(entries, 333), undefined);
  assert.equal(
    enrollmentForDevice(
      [...entries, { ...entries[1], alias: 'duplicate' }],
      id,
    ),
    undefined,
  );
  assert.equal(
    enrollmentForCard([...entries, { ...entries[1], alias: 'duplicate' }], 111),
    undefined,
  );
});

test('missing identities are not inferred and invalid identities are rejected', () => {
  const entries = decodeYubiAccounts([{ alias: 'pending', state: 'pending' }]);
  assert.equal(enrollmentForCard(entries, 111), undefined);
  assert.equal(enrollmentForDevice(entries, id), undefined);
  assert.throws(() =>
    decodeYubiAccounts([
      { alias: 'bad', state: 'complete', deviceId: '04' + 'ab'.repeat(32) },
    ]),
  );
  assert.throws(() =>
    decodeYubiAccounts([{ alias: 'bad', state: 'complete', cardSerial: 0 }]),
  );
});
