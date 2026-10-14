/**
 * Organising Files rows: the destination checks of the move sheet and of a
 * drag, the multi-selection operations, and the sequential bulk runner.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { FIXTURE } from '../src/fixture';
import type { AgentSnapshot, Item } from '../src/model';
import {
  NO_ROWS,
  allRows,
  bulkCounts,
  checkMove,
  checkMoveMany,
  dropVerdict,
  extendRows,
  folderExists,
  moveRowCheck,
  parentOf,
  pendingRows,
  runBulk,
  selectedRows,
  toggleRow,
  validFolderPath,
} from '../src/screens/item-moves';
import type { BulkAction, BulkRow } from '../src/screens/item-moves';

const STORE = 'acct:personal';

function item(path: string, kind: Item['kind'] = 'Secret', version = 1): Item {
  return {
    store: STORE,
    path,
    direntId: path.length.toString(16).padStart(32, '0'),
    kind,
    size: 4,
    version,
    read: 'Owner',
    write: 'Owner',
  };
}

function snapshotOf(items: Item[]): AgentSnapshot {
  return { ...FIXTURE, items };
}

const VAULT = snapshotOf([
  item('/env', 'Folder'),
  item('/env/token'),
  item('/env/prod', 'Folder'),
  item('/logins', 'Folder'),
  item('/logins/github.com'),
  item('/loose-a'),
  item('/loose-b'),
  item('/notes.txt', 'File'),
]);

test('paths: parents, and folder paths without empty, “.” or “..” parts', () => {
  assert.equal(parentOf('/a'), '/');
  assert.equal(parentOf('/a/b/c'), '/a/b');
  assert.ok(validFolderPath('/'));
  assert.ok(validFolderPath('/env/prod'));
  for (const bad of ['', 'env', '/env/', '/env//prod', '/./x', '/x/..'])
    assert.equal(validFolderPath(bad), false, bad);
});

test('a folder exists when the catalog lists it, holds entries under it, or was just created', () => {
  assert.ok(folderExists(VAULT, STORE, '/'));
  assert.ok(folderExists(VAULT, STORE, '/env/prod'));
  assert.ok(folderExists(VAULT, STORE, '/logins'));
  assert.equal(folderExists(VAULT, STORE, '/new'), false);
  assert.ok(folderExists(VAULT, STORE, '/new', ['/new']));
  assert.equal(folderExists(VAULT, 'acct:other', '/env'), false);
});

test('one item: rename, move, a folder to create, and the refusals', () => {
  const loose = item('/loose-a');
  assert.deepEqual(checkMove(VAULT, loose, '/loose-c'), {
    ok: true,
    summary: 'Renames loose-a to loose-c',
    creates: null,
  });
  assert.deepEqual(checkMove(VAULT, loose, '/env/prod/loose-a'), {
    ok: true,
    summary: 'Moves loose-a to /env/prod/loose-a',
    creates: null,
  });
  assert.deepEqual(checkMove(VAULT, loose, '/archive/2024/loose-a'), {
    ok: true,
    summary: 'Creates /archive/2024 and moves loose-a to /archive/2024/loose-a',
    creates: '/archive/2024',
  });
  const refusal = (destination: string, subject = loose): string => {
    const check = checkMove(VAULT, subject, destination);
    assert.equal(check.ok, false, destination);
    return check.ok ? '' : check.problem;
  };
  assert.match(refusal('/loose-a'), /another folder or change the name/);
  assert.match(refusal('/loose-b'), /loose-b is already at the top level/);
  assert.match(refusal('relative'), /starts with \//);
  assert.match(refusal('/.trash/owner/loose-a'), /Move to Trash/);
  // The VAULT has no Trash yet; its name stays reserved for one.
  assert.match(refusal('/.trash'), /Move to Trash/);
  assert.match(refusal('/.trash', item('/env/token')), /Move to Trash/);
  assert.match(refusal('/notes.txt/loose-a'), /\/notes\.txt is an item/);
  assert.match(
    refusal('/env/prod/env', item('/env', 'Folder')),
    /cannot move into itself/,
  );
});

test('several items: one store, a folder that is not theirs already, and what is already there', () => {
  const rows = [item('/loose-a'), item('/loose-b'), item('/env/token')];
  assert.deepEqual(checkMoveMany(VAULT, rows, '/env'), {
    ok: true,
    summary: 'Moves 2 items to /env; 1 already is there',
    creates: null,
  });
  assert.deepEqual(checkMoveMany(VAULT, rows, '/imported'), {
    ok: true,
    summary: 'Creates /imported and moves 3 items to /imported',
    creates: '/imported',
  });
  const refused = (items: readonly Item[], folder: string): string => {
    const check = checkMoveMany(VAULT, items, folder);
    assert.equal(check.ok, false, folder);
    return check.ok ? '' : check.problem;
  };
  assert.match(
    refused([item('/loose-a'), item('/loose-b')], '/'),
    /All 2 are already at the top level/,
  );
  assert.match(
    refused([item('/loose-a'), { ...item('/x'), store: 'team:eng' }], '/env'),
    /within their own vault/,
  );
  assert.match(refused(rows, '/.trash'), /Move to Trash/);
});

test('a drop names the move, or why the folder does not take it', () => {
  const rows = [item('/loose-a'), item('/loose-b')];
  assert.deepEqual(dropVerdict(rows, STORE, '/env', 'Personal', null), {
    ok: true,
    label: 'Move 2 items to /env',
  });
  assert.deepEqual(
    dropVerdict([item('/env/token')], STORE, '/', 'Personal', null),
    { ok: true, label: 'Move token to the top level of Personal' },
  );
  assert.equal(dropVerdict(rows, 'team:eng', '/', 'Personal', null).ok, false);
  assert.equal(dropVerdict(rows, STORE, '/', 'Personal', null).ok, false);
  assert.deepEqual(
    dropVerdict(rows, STORE, '/env', 'Personal', 'Read-only for now.'),
    { ok: false, label: 'Read-only for now.' },
  );
});

test('the multi-selection toggles, extends from its anchor, and keeps list order', () => {
  const listed = ['a', 'b', 'c', 'd', 'e'];
  let rows = toggleRow(NO_ROWS, 'c', 'a');
  // The row the details panel showed joins the first ⌘-click.
  assert.deepEqual(rows, { keys: ['a', 'c'], anchor: 'c' });
  rows = extendRows(rows, listed, 'e');
  assert.deepEqual(rows, { keys: ['c', 'd', 'e'], anchor: 'c' });
  rows = extendRows(rows, listed, 'a');
  assert.deepEqual(rows.keys, ['a', 'b', 'c']);
  // ⌘-click moves the anchor to the row it toggled, also when it removes it.
  rows = toggleRow(rows, 'a');
  assert.deepEqual(rows, { keys: ['b', 'c'], anchor: 'a' });
  // Shift-click with no anchor starts from the details panel's row.
  assert.deepEqual(extendRows(NO_ROWS, listed, 'd', 'b').keys, ['b', 'c', 'd']);
  assert.deepEqual(allRows(listed).keys, listed);
  const items = [item('/x'), item('/y'), item('/z')];
  assert.deepEqual(
    selectedRows(
      { keys: [`${STORE}|/z`, `${STORE}|/gone`, `${STORE}|/x`], anchor: null },
      items,
    ).map((entry) => entry.path),
    ['/x', '/z'],
  );
});

test('a row already in the folder is skipped and a taken name is refused', () => {
  assert.deepEqual(moveRowCheck(item('/env/token'), VAULT, '/env'), {
    status: 'skipped',
    message: 'Already in /env',
  });
  assert.equal(
    moveRowCheck(item('/x/github.com'), VAULT, '/logins')?.status,
    'refused',
  );
  assert.equal(moveRowCheck(item('/loose-a'), VAULT, '/env'), null);
});

function moveAction(
  sent: Item[],
  failures: Map<string, unknown> = new Map(),
): BulkAction {
  return {
    running: 'Moving…',
    check: (entry, snapshot) => moveRowCheck(entry, snapshot, '/env'),
    apply: async (entry) => {
      const failure = failures.get(entry.path);
      if (failure) {
        failures.delete(entry.path);
        throw failure;
      }
      sent.push(entry);
      return 'Moved to /env';
    },
  };
}

test('the runner reads the catalog before each item and sends its current version', async () => {
  const sent: Item[] = [];
  let reads = 0;
  const rows = pendingRows([item('/loose-a'), item('/loose-b')]);
  const seen: (readonly BulkRow[])[] = [];
  const finished = await runBulk({
    rows,
    indexes: [0, 1],
    action: moveAction(sent),
    reload: async () => {
      reads++;
      // Another session edited loose-b: same entry, newer version.
      return snapshotOf(
        VAULT.items.map((entry) =>
          entry.path === '/loose-b' ? { ...entry, version: 7 } : entry,
        ),
      );
    },
    onRows: (next) => seen.push(next),
    onFailure: async () => {},
  });
  assert.equal(reads, 2);
  assert.deepEqual(
    sent.map((entry) => [entry.path, entry.version]),
    [
      ['/loose-a', 1],
      ['/loose-b', 7],
    ],
  );
  assert.deepEqual(bulkCounts(finished).done, 2);
  assert.ok(seen.some((step) => step[1]?.status === 'running'));
});

test('the runner classifies failures by whether the request may have run', async () => {
  const failures = new Map<string, unknown>([
    [
      '/loose-a',
      {
        code: 'mutation-in-flight',
        message: 'Another change is in progress.',
        retryable: true,
        ambiguous: false,
        fatal: false,
      },
    ],
    [
      '/loose-b',
      {
        code: 'agent-lost',
        message: 'The agent stopped.',
        retryable: false,
        ambiguous: true,
        fatal: false,
      },
    ],
    [
      '/notes.txt',
      {
        code: 'conflict',
        message: 'This item changed.',
        retryable: false,
        ambiguous: false,
        fatal: false,
      },
    ],
  ]);
  const reported: string[] = [];
  const finished = await runBulk({
    rows: pendingRows([
      item('/loose-a'),
      item('/loose-b'),
      item('/notes.txt', 'File'),
      item('/env/token'),
      item('/removed'),
    ]),
    indexes: [0, 1, 2, 3, 4],
    action: moveAction([], failures),
    reload: async () => VAULT,
    onRows: () => {},
    onFailure: async (_, entry) => {
      reported.push(entry.path);
    },
  });
  assert.deepEqual(
    finished.map((row) => row.status),
    ['failed', 'uncertain', 'refused', 'skipped', 'refused'],
  );
  assert.match(finished[4]?.message ?? '', /changed or removed elsewhere/);
  assert.deepEqual(reported, ['/loose-a', '/loose-b', '/notes.txt']);
});

test('the runner stops when the catalog cannot be read, leaving the rest not started', async () => {
  let reads = 0;
  const sent: Item[] = [];
  const finished = await runBulk({
    rows: pendingRows([item('/loose-a'), item('/loose-b'), item('/notes.txt')]),
    indexes: [0, 1, 2],
    action: moveAction(sent),
    reload: async () => {
      if (++reads > 1)
        throw {
          code: 'io',
          message: 'The catalog could not be read.',
          retryable: true,
          ambiguous: false,
          fatal: false,
        };
      return VAULT;
    },
    onRows: () => {},
    onFailure: async () => {},
  });
  assert.equal(sent.length, 1);
  assert.deepEqual(
    finished.map((row) => row.status),
    ['done', 'failed', 'failed'],
  );
  assert.match(finished[1]?.message ?? '', /^Not started:/);
});
