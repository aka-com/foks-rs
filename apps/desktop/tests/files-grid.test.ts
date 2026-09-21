import assert from 'node:assert/strict';
import test from 'node:test';
import {
  filesGridWindow,
  FILES_CARD_HEIGHT,
  FILES_CARD_HEIGHT_WITH_LOCATION,
  FILES_CARD_GAP,
} from '../src/screens/files-grid';
import { storedFilesView, rememberFilesView } from '../src/files-view-pref';
import { installDom } from './lib/dom-harness';

const viewport = { listTop: 0, scrollTop: 0, viewport: 400, overscan: 1 };

test('grid resizing windows complete rows and keeps the final partial row reachable', () => {
  for (const [location, cardHeight] of [
    [false, FILES_CARD_HEIGHT],
    [true, FILES_CARD_HEIGHT_WITH_LOCATION],
  ] as const) {
    for (const [width, columns] of [
      [120, 1],
      [329, 1],
      [330, 2],
      [500, 3],
      [1000, 5],
    ]) {
      const count = 103;
      const first = filesGridWindow({ ...viewport, width, count, location });
      assert.equal(first.columns, columns);
      assert.equal(first.start, 0);
      assert.equal(first.end % columns, 0);
      assert.ok(first.end < count);
      const totalRows = Math.ceil(count / columns);
      const totalHeight =
        totalRows * (cardHeight + FILES_CARD_GAP) - FILES_CARD_GAP;
      for (const scrollTop of [0, 1200, 100_000]) {
        const range = filesGridWindow({
          ...viewport,
          width,
          count,
          location,
          scrollTop,
        });
        assert.equal(range.start % columns, 0);
        const mountedRows = Math.ceil((range.end - range.start) / columns);
        const mountedHeight =
          mountedRows * (cardHeight + FILES_CARD_GAP) -
          FILES_CARD_GAP +
          range.trailingGap;
        assert.equal(
          range.padTop + mountedHeight + range.padBottom,
          totalHeight,
        );
        assert.ok(range.end > range.start);
        if (scrollTop === 100_000) {
          assert.equal(range.end, count);
          assert.equal(range.padBottom, 0);
          assert.equal(range.trailingGap, 0);
        }
      }
    }
  }
});

test('grid accounts for content above it and empty results', () => {
  const a = filesGridWindow({
    ...viewport,
    count: 100,
    width: 600,
    scrollTop: 1000,
  });
  const b = filesGridWindow({
    ...viewport,
    count: 100,
    width: 600,
    scrollTop: 1100,
    listTop: 100,
  });
  assert.deepEqual(a, b);
  const empty = filesGridWindow({ ...viewport, count: 0, width: 0 });
  assert.deepEqual(empty, {
    start: 0,
    end: 0,
    padTop: 0,
    padBottom: 0,
    columns: 1,
    trailingGap: 0,
  });
});

test('view preference defaults to list, persists choices, and tolerates unavailable storage', () => {
  const dom = installDom();
  assert.equal(storedFilesView(), 'list');
  rememberFilesView('grid');
  assert.equal(storedFilesView(), 'grid');
  rememberFilesView('list');
  assert.equal(storedFilesView(), 'list');
  window.localStorage.setItem('filesView', 'unexpected');
  assert.equal(storedFilesView(), 'list');
  Object.defineProperty(window, 'localStorage', {
    configurable: true,
    get() {
      throw new Error('Storage blocked');
    },
  });
  assert.equal(storedFilesView(), 'list');
  assert.doesNotThrow(() => rememberFilesView('grid'));
  dom.window.close();
});
