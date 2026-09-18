import assert from 'node:assert/strict';
import test from 'node:test';
import { CatalogReadGate } from '../src/catalog-read-gate';
const flush = async () => {
  for (let i = 0; i < 20; i++) await Promise.resolve();
};
function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}
test('full reads wait for active profiles and take precedence over new background reads', async () => {
  const gate = new CatalogReadGate(),
    blocked = deferred(),
    order: string[] = [];
  const first = gate.profile(async () => {
    order.push('p');
    await blocked.promise;
  });
  const second = gate.profile(async () => {
    order.push('q');
  });
  await flush();
  const full = gate.exclusive(async () => {
    order.push('full');
  });
  const third = gate.profile(async () => {
    order.push('r');
  });
  await flush();
  assert.deepEqual(order, ['p', 'q']);
  blocked.resolve();
  await Promise.all([first, second, full, third]);
  assert.deepEqual(order, ['p', 'q', 'full', 'r']);
});
test('failed reads release their reservations', async () => {
  const gate = new CatalogReadGate();
  await assert.rejects(
    gate.profile(async () => {
      throw new Error('profile');
    }),
  );
  await assert.rejects(
    gate.exclusive(async () => {
      throw new Error('full');
    }),
  );
  assert.equal(await gate.profile(async () => 3), 3);
  assert.equal(await gate.exclusive(async () => 4), 4);
});
