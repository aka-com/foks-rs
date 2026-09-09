/**
 * Renders and verifies the primary application shell using jsdom and Vite SSR loader.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';

const dom = installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});

type TestingLibrary = typeof import('@testing-library/react');
let testingLibrary: TestingLibrary;
let vite: ViteDevServer;

test.before(async () => {
  testingLibrary = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  await vite.ssrLoadModule('/app.tsx');
  await testingLibrary.waitFor(() => {
    assert.ok(document.querySelector('.app'));
    assert.equal(document.querySelector('.app-loading'), null);
  });
});

test.after(async () => {
  await vite.close();
});

test('renders shell frame with application brand name', () => {
  assert.ok(document.querySelector('.window'), 'window container element should exist');
  assert.equal(document.querySelector('.titlebar .brand')?.textContent, 'FOKS');
  assert.equal(
    document.querySelectorAll('.web-mock-window .lights .light').length,
    3,
  );
  // Verify main navigation landmark exists.
  assert.ok(document.querySelector('nav.side[aria-label="Main Navigation"]'));
  assert.ok(document.querySelector('main.main'));
  // Brand name is rendered in the title bar rather than sidebar navigation.
  assert.equal(document.querySelector('.side .appname'), null);
});

test('the sidebar lists configured vaults and groups', () => {
  const captions = [...document.querySelectorAll('.side .nav .t')].map((node) =>
    node.textContent?.trim(),
  );
  assert.ok(captions.some((text) => text?.startsWith('All items')));
  assert.ok(captions.some((text) => text?.startsWith('Personal')));
  assert.ok(captions.some((text) => text?.startsWith('Work (Acme)')));
  // Verify roster summary caption is formatted from active group members.
  assert.ok(
    captions.some((text) => text?.includes('5 people · 1 group')),
    `Engineering's roster summary is on the row: ${captions.join(' | ')}`,
  );
});

test('displays active warning badge count in navigation', () => {
  // Active warnings in default state reflect non-critical server alerts.
  assert.equal(document.querySelector('.side .badge')?.textContent, '2');
});

test('renders navigation icons as SVG elements', () => {
  const icon = document.querySelector('.side .nav .ic');
  assert.ok(icon, 'a nav row has an icon');
  assert.equal(icon.getAttribute('viewBox'), '0 0 24 24');
  assert.equal(icon.getAttribute('stroke'), 'currentColor');
  assert.equal(icon.getAttribute('fill'), 'none');
  assert.equal(icon.getAttribute('stroke-width'), '1.7');
  assert.ok(icon.children.length > 0, 'icon contains SVG child elements');
});

test('defaults to All Items view on initial load', () => {
  // Without query state, initial navigation defaults to All Items.
  assert.equal(document.querySelector('.loc h1')?.textContent, 'All items');
  const rows = document.querySelectorAll('.body .row');
  assert.ok(rows.length > 0, 'items list is rendered in main body');
  assert.equal(document.querySelector('[data-shell-placeholder]'), null);
});

test('clicking a group navigates the shell to it', async () => {
  const rows = [...document.querySelectorAll<HTMLButtonElement>('.side .nav')];
  // Locate group navigation item by name.
  const engineering = rows.find((row) =>
    row.querySelector('.t')?.textContent?.startsWith('Engineering'),
  );
  assert.ok(engineering, 'the Engineering row is in the sidebar');
  const icon = engineering.querySelector('.ic');
  const stack = engineering.querySelector('.stack');
  assert.ok(icon, 'group icon is present');
  assert.ok(stack, 'roster stack element is present');
  assert.equal(
    Boolean(
      icon.compareDocumentPosition(stack) & Node.DOCUMENT_POSITION_FOLLOWING,
    ),
    true,
    'group icon precedes roster avatars in DOM order',
  );

  testingLibrary.fireEvent.click(engineering);
  await testingLibrary.waitFor(() => {
    assert.equal(document.querySelector('.loc h1')?.textContent, 'Engineering');
  });
  assert.equal(engineering.className.includes('on'), true);
});

test.after(() => {
  dom.window.close();
});
