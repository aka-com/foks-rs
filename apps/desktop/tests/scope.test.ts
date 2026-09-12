/** Tests for derived folder navigation over the item catalog. */

import assert from 'node:assert/strict';
import test from 'node:test';

import type { Item } from '../src/model';
import { folderAt, folderTree } from '../src/screens/scope';

const item = (path: string): Item => ({
  store: 'acct:personal',
  path,
  kind: 'Secret',
  size: 1,
  version: 1,
  read: { role: 'Owner' },
  write: { role: 'Owner' },
});

test('folder tree derives sorted folders, direct items, and recursive counts', () => {
  const root = folderTree([
    item('/z-last'),
    item('/env/prod/token'),
    item('/agents/key'),
    item('/env/readme'),
    item('/env/prod/cert'),
  ]);

  assert.equal(root.count, 5);
  assert.deepEqual(
    root.folders.map((folder) => folder.path),
    ['/agents', '/env'],
  );
  assert.deepEqual(
    root.items.map((entry) => entry.path),
    ['/z-last'],
  );

  const env = folderAt(root, '/env');
  assert.ok(env);
  assert.equal(env.count, 3);
  assert.deepEqual(
    env.items.map((entry) => entry.path),
    ['/env/readme'],
  );

  const prod = folderAt(root, '/env/prod');
  assert.ok(prod);
  assert.equal(prod.count, 2);
  assert.deepEqual(
    prod.items.map((entry) => entry.path),
    ['/env/prod/token', '/env/prod/cert'],
  );
});

test('folder lookup returns the root for empty paths and null for stale paths', () => {
  const root = folderTree([item('/agents/key')]);
  assert.equal(folderAt(root, ''), root);
  assert.equal(folderAt(root, '/'), root);
  assert.equal(folderAt(root, '/missing'), null);
});
