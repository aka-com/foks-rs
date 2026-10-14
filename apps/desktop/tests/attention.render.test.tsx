/**
 * The header's attention bell in the mounted shell: it lists what the rail's
 * signals stand for, one entity at a time, links each to its page, carries
 * the setup steps still open, and lists the session's toast messages.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import type { Bridge } from '../src/bridge';

installDom({
  url: 'http://localhost/?state=files',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});
let vite: ViteDevServer;
let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
});
test.afterEach(() => {
  ui.cleanup();
  window.localStorage.clear();
  Reflect.deleteProperty(document, 'hidden');
});
test.after(async () => vite.close());

const UNAVAILABLE = {
  locked: false,
  available: false,
  mechanism: 'none' as const,
  unavailableReason: 'polkit is not installed',
};

function panel(): HTMLElement {
  const found = document.getElementById('attention-centre');
  assert.ok(found, 'the attention popover is open');
  return found;
}

async function openBell(): Promise<HTMLElement> {
  const bell = document.querySelector<HTMLButtonElement>('.attention-trigger');
  assert.ok(bell);
  if (bell.getAttribute('aria-expanded') !== 'true')
    await ui.act(async () => {
      ui.fireEvent.click(bell);
    });
  return panel();
}

test('the bell lists each signal, links it to its page, and keeps the setup steps and recent messages', async () => {
  // The shell reads device metadata only while the window is visible.
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    value: false,
  });
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { deviceAlertRegistry } = (await vite.ssrLoadModule(
    '/src/screens/device-alert.ts',
  )) as typeof import('../src/screens/device-alert');
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base,
    appLockState: async () => UNAVAILABLE,
    lockApp: async () => UNAVAILABLE,
  };
  ui.render(createElement(App, { bridge }));
  // The shell preloads each account's keys; the work account has one device
  // and no recovery phrase.
  await ui.waitFor(() => {
    const facts = deviceAlertRegistry(bridge).getSnapshot();
    assert.equal(facts.phraseLists.get('acct:work')?.length, 0);
    assert.deepEqual(facts.keyCounts.get('acct:work'), {
      devices: 1,
      cards: 0,
    });
  });
  await ui.waitFor(() => {
    const bell = document.querySelector('.attention-trigger');
    assert.match(bell?.getAttribute('aria-label') ?? '', /need attention$/);
  });

  let open = await openBell();
  const rows = [...open.querySelectorAll('.attention-row')].map(
    (row) => row.textContent ?? '',
  );
  assert.ok(rows.some((text) => /Server not verified/.test(text)));
  assert.ok(
    rows.some((text) => /No recovery phrase.*vitalik on Acme/.test(text)),
  );
  assert.ok(
    rows.some((text) =>
      /To do: Add a second device or a hardware key.*vitalik on Acme has one device/.test(
        text,
      ),
    ),
  );
  assert.ok(
    rows.some((text) => /To do: App lock.*polkit is not installed/.test(text)),
  );
  assert.equal(
    open.querySelector('.attention-recent'),
    null,
    'no message has been shown yet',
  );

  // An item is a link to the page that resolves it.
  const missing = [
    ...open.querySelectorAll<HTMLButtonElement>('button.attention-row'),
  ].find((row) => /No recovery phrase/.test(row.textContent ?? ''));
  assert.ok(missing);
  await ui.act(async () => {
    ui.fireEvent.click(missing);
  });
  assert.equal(document.getElementById('attention-centre'), null);
  assert.match(window.location.search, /state=devices/);
  assert.match(decodeURIComponent(window.location.search), /acct:work/);

  // A toast's message stays in Recent after the toast has gone.
  await ui.act(async () => {
    ui.fireEvent.click(document.querySelector('.rail-foot .who')!);
  });
  await ui.act(async () => {
    ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Lock' }));
  });
  await ui.waitFor(() =>
    assert.ok(
      document.body.textContent?.includes(
        'Application lock is not available on this system.',
      ),
    ),
  );
  open = await openBell();
  assert.deepEqual(
    [...open.querySelectorAll('.attention-recent .t')].map(
      (node) => node.textContent,
    ),
    ['Application lock is not available on this system.'],
  );

  // Hiding the checklist is a stored preference.
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(open).getByRole('button', { name: 'Hide' }));
  });
  assert.equal(open.textContent?.includes('Get set up'), false);
  assert.equal(
    window.localStorage.getItem('onboarding.dismissed.setup-checklist'),
    'true',
  );
});
