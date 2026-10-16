import assert from 'node:assert/strict';
import test from 'node:test';
import {
  gridFocusTarget,
  revealOffset,
  rovingKey,
  treeEntries,
  treeKeyAction,
} from '../src/features/files/files-keyboard';
import type { FolderNode } from '../src/model/scope';

test('list keys move one row, a page, or to either end', () => {
  const at = (key: string, index: number) =>
    gridFocusTarget(key, { index, count: 50, columns: 1, page: 10 });
  assert.equal(at('ArrowDown', 0), 1);
  assert.equal(at('ArrowUp', 1), 0);
  assert.equal(at('ArrowUp', 0), null);
  assert.equal(at('ArrowDown', 49), null);
  assert.equal(at('Home', 30), 0);
  assert.equal(at('End', 3), 49);
  assert.equal(at('PageDown', 5), 15);
  assert.equal(at('PageDown', 45), 49);
  assert.equal(at('PageUp', 5), 0);
  // Left and Right belong to the row's own actions in the list view.
  assert.equal(at('ArrowRight', 3), null);
  assert.equal(at('ArrowLeft', 3), null);
  assert.equal(at('a', 3), null);
  assert.equal(
    gridFocusTarget('ArrowDown', { index: 0, count: 0, columns: 1, page: 1 }),
    null,
  );
});

test('grid keys move across a line and down by a whole line', () => {
  // 10 cards in lines of 4: the last line holds cards 8 and 9.
  const at = (key: string, index: number) =>
    gridFocusTarget(key, { index, count: 10, columns: 4, page: 2 });
  assert.equal(at('ArrowRight', 0), 1);
  assert.equal(at('ArrowRight', 3), 4);
  assert.equal(at('ArrowRight', 9), null);
  assert.equal(at('ArrowLeft', 4), 3);
  assert.equal(at('ArrowLeft', 0), null);
  assert.equal(at('ArrowDown', 1), 5);
  assert.equal(at('ArrowDown', 5), 9);
  // Below card 6 the short last line has nothing; its final card takes focus.
  assert.equal(at('ArrowDown', 6), 9);
  assert.equal(at('ArrowDown', 9), null);
  assert.equal(at('ArrowUp', 9), 5);
  assert.equal(at('ArrowUp', 2), null);
  assert.equal(at('PageDown', 0), 8);
  assert.equal(at('PageUp', 9), 1);
  assert.equal(at('End', 0), 9);
});

test('revealOffset scrolls only as far as the line needs', () => {
  const base = { height: 38, viewport: 200 };
  assert.equal(revealOffset({ ...base, top: 100, scrollTop: 50 }), 50);
  assert.equal(revealOffset({ ...base, top: 20, scrollTop: 50 }), 20);
  assert.equal(revealOffset({ ...base, top: 300, scrollTop: 50 }), 138);
  // A sticky header covers the top of the viewport.
  assert.equal(
    revealOffset({ ...base, top: 60, scrollTop: 50, stickyTop: 28 }),
    32,
  );
  assert.equal(
    revealOffset({ ...base, top: 10, scrollTop: 50, stickyTop: 28 }),
    0,
  );
});

test('rovingKey keeps the first preferred key that is still drawn', () => {
  const keys = ['a', 'b', 'c'];
  assert.equal(rovingKey(keys, 'z', 'b'), 'b');
  assert.equal(rovingKey(keys, null, undefined), 'a');
  assert.equal(rovingKey(keys, 'c', 'b'), 'c');
  assert.equal(rovingKey([], 'a'), undefined);
});

function node(path: string, folders: FolderNode[] = []): FolderNode {
  return {
    name: path.split('/').at(-1) ?? '',
    path,
    folders,
    items: [],
    count: 0,
  };
}

const ROOTS = [
  {
    storeId: 's1',
    root: node('/', [node('/a', [node('/a/x')]), node('/b')]),
    trash: true,
  },
  { storeId: 's2', root: node('/'), trash: false },
];

test('treeEntries lists drawn rows with their levels and fold state', () => {
  const open = treeEntries(ROOTS, new Set(), '/.trash');
  assert.deepEqual(
    open.map((entry) => [entry.key, entry.level, entry.expandable]),
    [
      ['s1|/', 1, true],
      ['s1|/a', 2, true],
      ['s1|/a/x', 3, false],
      ['s1|/b', 2, false],
      ['s1|/.trash', 2, false],
      ['s2|/', 1, false],
    ],
  );
  const folded = treeEntries(ROOTS, new Set(['s1|/a']), '/.trash');
  assert.deepEqual(
    folded.map((entry) => entry.key),
    ['s1|/', 's1|/a', 's1|/b', 's1|/.trash', 's2|/'],
  );
  assert.equal(folded[1].open, false);
  // A closed store hides its folders and its Trash.
  assert.deepEqual(
    treeEntries(ROOTS, new Set(['s1|/']), '/.trash').map((entry) => entry.key),
    ['s1|/', 's2|/'],
  );
});

test('tree keys open, close, enter and leave folders', () => {
  const open = treeEntries(ROOTS, new Set(), '/.trash');
  assert.deepEqual(treeKeyAction('ArrowDown', open, 0), {
    kind: 'focus',
    index: 1,
  });
  assert.equal(treeKeyAction('ArrowDown', open, 5), null);
  assert.equal(treeKeyAction('ArrowUp', open, 0), null);
  assert.deepEqual(treeKeyAction('End', open, 0), { kind: 'focus', index: 5 });
  assert.deepEqual(treeKeyAction('Home', open, 4), { kind: 'focus', index: 0 });
  // Right on an open folder enters it; Left on it closes it.
  assert.deepEqual(treeKeyAction('ArrowRight', open, 1), {
    kind: 'focus',
    index: 2,
  });
  assert.deepEqual(treeKeyAction('ArrowLeft', open, 1), {
    kind: 'toggle',
    index: 1,
  });
  // Left on a leaf, or on Trash, moves to the parent.
  assert.deepEqual(treeKeyAction('ArrowLeft', open, 2), {
    kind: 'focus',
    index: 1,
  });
  assert.deepEqual(treeKeyAction('ArrowLeft', open, 4), {
    kind: 'focus',
    index: 0,
  });
  assert.equal(treeKeyAction('ArrowLeft', open, 5), null);
  assert.equal(treeKeyAction('ArrowRight', open, 3), null);
  const folded = treeEntries(ROOTS, new Set(['s1|/a']), '/.trash');
  assert.deepEqual(treeKeyAction('ArrowRight', folded, 1), {
    kind: 'toggle',
    index: 1,
  });
  assert.deepEqual(treeKeyAction('Enter', folded, 2), {
    kind: 'select',
    index: 2,
  });
  assert.deepEqual(treeKeyAction(' ', folded, 2), {
    kind: 'select',
    index: 2,
  });
  assert.equal(treeKeyAction('ArrowDown', folded, 9), null);
});

test('a store whose only child is its Trash opens, closes and is entered', () => {
  const roots = [{ storeId: 's3', root: node('/'), trash: true }];
  const open = treeEntries(roots, new Set(), '/.trash');
  assert.deepEqual(
    open.map((entry) => [entry.key, entry.level, entry.expandable]),
    [
      ['s3|/', 1, true],
      ['s3|/.trash', 2, false],
    ],
  );
  assert.deepEqual(treeKeyAction('ArrowRight', open, 0), {
    kind: 'focus',
    index: 1,
  });
  assert.deepEqual(treeKeyAction('ArrowLeft', open, 0), {
    kind: 'toggle',
    index: 0,
  });
  const closed = treeEntries(roots, new Set(['s3|/']), '/.trash');
  assert.deepEqual(
    closed.map((entry) => entry.key),
    ['s3|/'],
  );
  assert.deepEqual(treeKeyAction('ArrowRight', closed, 0), {
    kind: 'toggle',
    index: 0,
  });
});
