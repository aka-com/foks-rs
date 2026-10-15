/**
 * The Files list's column header and subfolder row rendered from their own
 * module under screens/files, without the screen around them: which column
 * states the sort, the order a header click asks for, and how a subfolder row
 * sits in the grid.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';

import type { SortDirection, SortKey } from '../src/location';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');

test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

test('within one vault the Name header states a sort by location, and a click reverses it', async () => {
  const { ListHeader } = await import('../src/screens/files/list-rows');
  const asked: [SortKey, SortDirection][] = [];
  const rendered = ui.render(
    createElement(ListHeader, {
      columns: { location: false },
      sort: 'group',
      direction: 'asc',
      onSort: (sort, direction) => asked.push([sort, direction]),
    }),
  );
  const headers = rendered.getAllByRole('columnheader');
  assert.deepEqual(
    headers.map((header) => header.textContent),
    ['Name ↑', 'Kind', 'Size', 'Actions'],
  );
  assert.equal(headers[0].getAttribute('aria-sort'), 'ascending');
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Name ↑' }));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Kind' }));
  assert.deepEqual(asked, [
    ['name', 'desc'],
    ['kind', 'asc'],
  ]);
});

test('a subfolder row is a grid row with one cell per column, and Enter opens it', async () => {
  const { FolderRow } = await import('../src/screens/files/list-rows');
  let opened = 0;
  const rendered = ui.render(
    createElement(FolderRow, {
      nav: { layout: 'list', key: 'folder', tabIndex: 0, rowIndex: 2 },
      storeId: 'acct:personal',
      path: '/logins',
      name: 'logins',
      count: 2,
      columns: { location: true },
      onSelect: () => {
        opened += 1;
      },
    }),
  );
  const row = rendered.getByRole('row');
  assert.equal(row.getAttribute('data-folder-path'), '/logins');
  assert.equal(row.getAttribute('aria-rowindex'), '2');
  assert.equal(ui.within(row).getAllByRole('gridcell').length, 5);
  ui.fireEvent.keyDown(row, { key: 'Enter' });
  assert.equal(opened, 1);
});
