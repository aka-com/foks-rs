/**
 * The copies a Files row, its menu and the ⌘K palette offer for an item,
 * and the toast that reports one.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { FIXTURE } from '../src/fixture';
import type { Item } from '../src/model';
import type { Bridge } from '../src/bridge';
import type { ToastOptions } from '../kit/toasts';
import {
  COPY_LABELS,
  copyActions,
  copyAndAnnounce,
} from '../src/screens/item-copy';

function itemAt(path: string): Item {
  const item = FIXTURE.items.find((candidate) => candidate.path === path);
  assert.ok(item, path);
  return item;
}

function fieldsOf(item: Item): string[] {
  return copyActions(item).map(({ field }) => field);
}

test('a listed login copies its password and user name', () => {
  const login = itemAt('/logins/github.com');
  assert.equal(login.shape, 'login');
  assert.deepEqual(fieldsOf(login), ['password', 'username']);
});

test('a Password with no listed shape also copies its whole value', () => {
  const unshaped: Item = { ...itemAt('/logins/github.com') };
  delete unshaped.shape;
  assert.deepEqual(fieldsOf(unshaped), ['password', 'username', 'value']);
});

test('a text Document copies its value and a file copies nothing', () => {
  assert.deepEqual(fieldsOf(itemAt('/env/prod/DATABASE_URL')), ['value']);
  const file = FIXTURE.items.find(({ kind }) => kind === 'File');
  assert.ok(file);
  assert.deepEqual(fieldsOf(file), []);
});

test('every copy is labelled from the one field-to-label map', () => {
  for (const item of FIXTURE.items)
    for (const { field, label } of copyActions(item))
      assert.equal(label, COPY_LABELS[field]);
});

test('the copy toast closes by the time the host clears the clipboard', async (t) => {
  let now = 50_000;
  t.mock.method(Date, 'now', () => now);
  const bridge = {
    copyItemField: async () => {
      // The host's clear timer starts during the call.
      now += 200;
      return { ok: true, clearAfterSeconds: 15 };
    },
  } as unknown as Bridge;
  const shown: [string, ToastOptions | undefined][] = [];
  await copyAndAnnounce({
    bridge,
    item: itemAt('/logins/github.com'),
    field: 'password',
    toasts: { show: (message, options) => shown.push([message, options]) },
    onError: (error) => assert.fail(String(error)),
  });
  assert.deepEqual(shown, [
    ['Password copied · clears in 15 s', { expiresAt: 50_000 + 15_000 }],
  ]);
});
