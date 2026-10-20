/**
 * The details panel's logic that needs no React: the rows a login lists, the
 * lines its structured editor opens with, and the shared item read with its
 * exact-version check. Loaded in node with no DOM.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import type { Bridge, ItemRequest, ReadItemResponse } from '../src/bridge';
import {
  assertExactRead,
  fieldLabel,
  loginEditLines,
  loginRows,
  readOnce,
} from '../src/features/files/details-model';

test('an unread login lists its three fields, each without a value', () => {
  assert.deepEqual(loginRows(null), [
    { id: 'username', label: 'User name', field: 'username', value: null },
    { id: 'password', label: 'Password', field: 'password', value: null },
    { id: 'url', label: 'Website', field: 'url', value: null },
  ]);
});

test('a read login lists its fields, then every other line with a value in stored order', () => {
  const text = [
    'user: satoshi',
    'password: pale-orbit',
    'ssid: home',
    'a line without a key',
    '',
    'constructor: value',
    'note:',
  ].join('\n');
  assert.deepEqual(loginRows(text), [
    { id: 'username', label: 'User name', field: 'username', value: 'satoshi' },
    {
      id: 'password',
      label: 'Password',
      field: 'password',
      value: 'pale-orbit',
    },
    { id: 'url', label: 'Website', field: 'url', value: null },
    { id: 'line:2', label: 'Network', field: null, value: 'home' },
    { id: 'line:3', label: 'Text', field: null, value: 'a line without a key' },
    { id: 'line:5', label: 'constructor', field: null, value: 'value' },
  ]);
});

test('a key with no known label is its own label, never a prototype property', () => {
  assert.equal(fieldLabel('website'), 'Website');
  assert.equal(fieldLabel('toString'), 'toString');
});

test('the login editor opens only on text with a password line, and skips blank lines', () => {
  assert.equal(loginEditLines('user: satoshi\nurl: example.com'), null);
  assert.deepEqual(
    loginEditLines('user: satoshi\n\npassword: x\n  \nurl: example.com'),
    [0, 2, 4],
  );
});

/** A bridge whose reads wait until the test settles them. */
function pendingReads(): {
  bridge: Bridge;
  calls: ItemRequest[];
  settle: (index: number) => void;
} {
  const calls: ItemRequest[] = [];
  const resolvers: (() => void)[] = [];
  const bridge = {
    readItem: (request: ItemRequest) => {
      calls.push(request);
      return new Promise<ReadItemResponse>((resolve) => {
        resolvers.push(() =>
          resolve({
            store: request.storeId,
            path: request.path,
            version: request.version,
            value: `read ${calls.length}`,
          }),
        );
      });
    },
  } as unknown as Bridge;
  return { bridge, calls, settle: (index) => resolvers[index]() };
}

test('concurrent reads of one item version share a request until it settles', async () => {
  const reads = pendingReads();
  const session = {};
  const request: ItemRequest = {
    storeId: 'acct:personal',
    path: '/logins/github.com',
    version: 9,
    direntId: 'aa'.repeat(16),
  };
  const first = readOnce(reads.bridge, request, 1, session);
  const second = readOnce(reads.bridge, { ...request }, 1, session);
  assert.equal(first, second);
  // Another access generation, or another version, is its own read.
  void readOnce(reads.bridge, request, 2, session);
  void readOnce(reads.bridge, { ...request, version: 10 }, 1, session);
  assert.equal(reads.calls.length, 3);
  reads.settle(0);
  await first;
  // Once the first read settles, the next request reads again.
  void readOnce(reads.bridge, request, 1, session);
  assert.equal(reads.calls.length, 4);
});

test('a read answering for another item or version is refused', () => {
  const request: ItemRequest = {
    storeId: 'acct:personal',
    path: '/logins/github.com',
    version: 9,
    direntId: 'aa'.repeat(16),
  };
  const response: ReadItemResponse = {
    store: 'acct:personal',
    path: '/logins/github.com',
    version: 9,
    value: 'password: x',
  };
  assert.doesNotThrow(() => assertExactRead(request, response));
  assert.throws(
    () => assertExactRead(request, { ...response, version: 8 }),
    /different item or version/,
  );
  assert.throws(
    () => assertExactRead(request, { ...response, path: '/logins/other' }),
    /different item or version/,
  );
});
