/**
 * The shell mounts.
 *
 * Boots `app.tsx` the way `ui/tests/app-root.render.test.tsx` does — jsdom
 * plus Vite's `ssrLoadModule`, so the module graph, the aliases and the JSX
 * transform are the real ones rather than a test-only approximation — and
 * asserts the Phase 1 frame is there and navigable.
 *
 * There is no `window.__TAURI__` in this realm and no Tauri runtime, so
 * `bridge.ts` takes the mock branch on its own. That is the point: the same
 * branch the Playwright acceptance run takes.
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

test('the shell frame mounts with the app name in it', () => {
  assert.ok(document.querySelector('.window'), 'the window is drawn');
  assert.equal(document.querySelector('.titlebar .brand')?.textContent, 'FOKS');
  assert.equal(
    document.querySelectorAll('.web-mock-window .lights .light').length,
    3,
  );
  // The sidebar is a landmark, not a div: it is how the whole app is reached.
  assert.ok(document.querySelector('nav.side[aria-label="Places"]'));
  assert.ok(document.querySelector('main.main'));
  // `sidebar()` starts at All items — the app name is on the title bar, and
  // only the first-run mode of this sidebar writes it again.
  assert.equal(document.querySelector('.side .appname'), null);
});

test('the sidebar lists the fixture s vaults and groups', () => {
  const captions = [...document.querySelectorAll('.side .nav .t')].map((node) =>
    node.textContent?.trim(),
  );
  assert.ok(captions.some((text) => text?.startsWith('All items')));
  assert.ok(captions.some((text) => text?.startsWith('Personal')));
  assert.ok(captions.some((text) => text?.startsWith('Work (Acme)')));
  // "5 people · 1 group" is `peopleGroups` on the real roster, not a literal.
  assert.ok(
    captions.some((text) => text?.includes('5 people · 1 group')),
    `Engineering's roster summary is on the row: ${captions.join(' | ')}`,
  );
});

test('issues carries the badge for the notes that apply now', () => {
  // The fresh lease world: the two warnings, not the lapsed-lease critical.
  assert.equal(document.querySelector('.side .badge')?.textContent, '2');
});

test('the icons render as SVG children, not as injected markup', () => {
  const icon = document.querySelector('.side .nav .ic');
  assert.ok(icon, 'a nav row has an icon');
  assert.equal(icon.getAttribute('viewBox'), '0 0 24 24');
  assert.equal(icon.getAttribute('stroke'), 'currentColor');
  assert.equal(icon.getAttribute('fill'), 'none');
  assert.equal(icon.getAttribute('stroke-width'), '1.7');
  assert.ok(icon.children.length > 0, 'the icon has drawn children');
});

test('the boot address lands on All items, listing the whole catalog', () => {
  // No `?state=`, so the shell starts where `decodeScene` starts.
  assert.equal(document.querySelector('.loc h1')?.textContent, 'All items');
  const rows = document.querySelectorAll('.body .row');
  assert.ok(rows.length > 0, 'the list is the body, not a placeholder');
  assert.equal(document.querySelector('[data-shell-placeholder]'), null);
});

test('clicking a group navigates the shell to it', async () => {
  const rows = [...document.querySelectorAll<HTMLButtonElement>('.side .nav')];
  // The row's own text starts with its avatar stack's initials, so the name
  // is read off `.t` — the same span the captions above are read from.
  const engineering = rows.find((row) =>
    row.querySelector('.t')?.textContent?.startsWith('Engineering'),
  );
  assert.ok(engineering, 'the Engineering row is in the sidebar');

  testingLibrary.fireEvent.click(engineering);
  await testingLibrary.waitFor(() => {
    assert.equal(document.querySelector('.loc h1')?.textContent, 'Engineering');
  });
  assert.equal(engineering.className.includes('on'), true);
});

test.after(() => {
  dom.window.close();
});
