import assert from 'node:assert/strict';
import test from 'node:test';

import { FIXTURE } from '../src/fixture';
import {
  catalog,
  encodeOriginalPath,
  isTrashPath,
  originPath,
  ownersOnly,
  parseTrashName,
  restoredCopyPath,
  trashEntries,
  trashEntryExpired,
  trashEntryLabel,
  trashStamp,
  trashUnavailable,
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

test('an entry is owners-only by the read role of the Trash folder it is filed in', () => {
  const member = (path: string, kind: Item['kind'] = 'Secret'): Item => ({
    ...item(path, kind),
    store: 'team:household',
    read: 'Member · visibility 0',
    write: 'Member · visibility 0',
  });
  const snapshot: AgentSnapshot = {
    ...FIXTURE,
    items: [
      ...FIXTURE.items,
      member('/.trash', 'Folder'),
      { ...member('/.trash/owner', 'Folder'), read: 'Owner', write: 'Owner' },
      // A member-read item from an owner-only folder is filed by that folder.
      member(`/.trash/owner/${STAMP}~%2Fdeeds%2Fnote`),
      member('/.trash/member-0', 'Folder'),
      member(`/.trash/member-0/${STAMP}~%2Fwifi`),
    ],
  };
  const [first, second] = trashEntries(snapshot, 'team:household');
  assert.equal(first.folderRead, 'Owner');
  assert.equal(ownersOnly(first), true);
  assert.equal(second.folderRead, 'Member · visibility 0');
  assert.equal(ownersOnly(second), false);
});

test('origin files, expiry and an unavailable team Trash are read conservatively', () => {
  assert.equal(originPath('/notes/long'), '/notes/long');
  assert.equal(originPath('notes/long'), null);
  assert.equal(originPath('/a\n/b'), null);

  const [entry] = trashEntries(
    {
      ...FIXTURE,
      items: [item(`/.trash/owner/${STAMP}~%2Fa`), item('/.trash/owner/x')],
    },
    'acct:personal',
  ).filter((candidate) => candidate.deletedAt !== null);
  const day = 24 * 60 * 60 * 1000;
  assert.equal(trashEntryExpired(entry, AT + 30 * day, 30 * day), true);
  assert.equal(trashEntryExpired(entry, AT + 29 * day, 30 * day), false);

  // team:eng lists this account as an Admin and has no /.trash; owners and
  // accounts can always create it.
  assert.equal(trashUnavailable(FIXTURE, 'team:eng'), true);
  assert.equal(trashUnavailable(FIXTURE, 'team:household'), false);
  assert.equal(trashUnavailable(FIXTURE, 'acct:personal'), false);
  assert.equal(
    trashUnavailable(
      {
        ...FIXTURE,
        items: [
          ...FIXTURE.items,
          { ...item('/.trash', 'Folder'), store: 'team:eng' },
        ],
      },
      'team:eng',
    ),
    false,
  );
  // A roster that does not list this account says nothing either way.
  assert.equal(
    trashUnavailable(
      {
        ...FIXTURE,
        parties: FIXTURE.parties.filter(
          (party) => !(party.store === 'team:eng' && party.label === 'you'),
        ),
      },
      'team:eng',
    ),
    false,
  );
});
