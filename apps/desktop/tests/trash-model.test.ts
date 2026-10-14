import assert from 'node:assert/strict';
import test from 'node:test';

import { FIXTURE } from '../src/fixture';
import {
  catalog,
  encodeOriginalPath,
  isTrashPath,
  parseTrashName,
  restoredCopyPath,
  trashEntries,
  trashEntryLabel,
  trashStamp,
} from '../src/model';
import type { AgentSnapshot, Item } from '../src/model';

const STAMP = '20231114T221320';
const AT = Date.UTC(2023, 10, 14, 22, 13, 20);

function item(path: string, kind: Item['kind'] = 'Secret'): Item {
  return {
    store: 'acct:personal',
    path,
    direntId: path.length.toString(16).padStart(32, '0'),
    kind,
    size: 4,
    version: 1,
    read: 'Owner',
    write: 'Owner',
  };
}

test('trash names round-trip the deletion time and the original path the host encodes', () => {
  assert.equal(trashStamp(new Date(AT)), STAMP);
  const original = '/logins/50% off~deals/été\tnote';
  const encoded = encodeOriginalPath(original);
  // The same bytes the native host writes for this path.
  assert.equal(encoded, '%2Flogins%2F50%25 off~deals%2Fété%09note');
  assert.deepEqual(parseTrashName(`${STAMP}~${encoded}`), {
    kind: 'path',
    deletedAt: AT,
    original,
  });
  const hash = 'a'.repeat(32);
  assert.deepEqual(parseTrashName(`${STAMP}~#${hash}`), {
    kind: 'hashed',
    deletedAt: AT,
  });
  assert.deepEqual(parseTrashName(`${STAMP}~#${hash}.origin`), {
    kind: 'origin',
  });
  for (const foreign of [
    'github.com',
    '20231114X221320~%2Fa',
    `${STAMP}~a`,
    `${STAMP}~%2`,
    `${STAMP}~%FF`,
    `${STAMP}~#short`,
  ])
    assert.equal(parseTrashName(foreign), null, foreign);
});

test('Trash is excluded from the catalog and listed newest first per store', () => {
  const later = '20240101T000000';
  const earlier = '20230101T000000';
  const hash = 'b'.repeat(32);
  const snapshot: AgentSnapshot = {
    ...FIXTURE,
    items: [
      ...FIXTURE.items,
      item('/.trash', 'Folder'),
      item('/.trash/owner', 'Folder'),
      item(`/.trash/owner/${STAMP}~%2Flogins%2Fgithub.com`),
      item(`/.trash/owner/${later}~%2Ftax-2024`, 'Folder'),
      item(`/.trash/owner/${later}~%2Ftax-2024/receipt.pdf`, 'File'),
      item(`/.trash/owner/${earlier}~#${hash}`),
      item(`/.trash/owner/${earlier}~#${hash}.origin`),
      item('/.trash/owner/copied-by-hand'),
    ],
  };
  assert.ok(catalog(snapshot).length > 0);
  assert.ok(catalog(snapshot).every((entry) => !isTrashPath(entry.path)));
  assert.equal(isTrashPath('/.trashcan'), false);

  const entries = trashEntries(snapshot, 'acct:personal');
  assert.deepEqual(
    entries.map((entry) => [trashEntryLabel(entry), entry.count]),
    [
      ['tax-2024', 1],
      ['github.com', 0],
      [`${earlier}~#${hash}`, 0],
      ['copied-by-hand', 0],
    ],
  );
  assert.equal(entries[0].original, '/tax-2024');
  assert.equal(entries[0].folder, '/.trash/owner');
  assert.equal(
    entries[2].origin?.path,
    `/.trash/owner/${earlier}~#${hash}.origin`,
  );
  assert.equal(entries[2].original, null);
  assert.equal(entries[3].deletedAt, null);
  assert.deepEqual(trashEntries(snapshot, 'team:eng'), []);
});

test('a restored copy takes the first free "(restored)" name', () => {
  const taken = new Set([
    '/logins/github.com',
    '/logins/github.com (restored)',
  ]);
  assert.equal(
    restoredCopyPath('/logins/github.com', (path) => taken.has(path)),
    '/logins/github.com (restored 2)',
  );
  assert.equal(
    restoredCopyPath('/a', () => false),
    '/a (restored)',
  );
});
