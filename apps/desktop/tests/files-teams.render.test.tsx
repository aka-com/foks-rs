/**
 * The Files roots page and the Teams list.
 *
 * Both draw the store rows the rail used to draw, and both say a store's
 * abnormal state the same way: a chip at the end of the row and a dimmed row,
 * never a caption under the name. The rows are also the only way into an item
 * page and a group's settings now, so where they navigate is pinned here.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';
import type { Location } from '../src/location';
import type { AgentSnapshot } from '../src/model';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;

test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
});
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

/** The row a name is drawn in. */
function row(name: string): HTMLButtonElement {
  const node = [
    ...document.querySelectorAll<HTMLButtonElement>('.nav-rows .row'),
  ].find((candidate) => candidate.querySelector('.tt')?.textContent === name);
  assert.ok(node, `no row named ${name}`);
  return node;
}

async function fixture(): Promise<AgentSnapshot> {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  return FIXTURE;
}

async function files(onNavigate: (location: Location) => void = () => {}) {
  const { FilesScreen } = (await vite.ssrLoadModule(
    '/src/screens/files-screen.tsx',
  )) as typeof import('../src/screens/files-screen');
  const snapshot = await fixture();
  return ui.render(createElement(FilesScreen, { snapshot, onNavigate }));
}

async function teams(onNavigate: (location: Location) => void = () => {}) {
  const { TeamsScreen } = (await vite.ssrLoadModule(
    '/src/screens/teams-screen.tsx',
  )) as typeof import('../src/screens/teams-screen');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { ToastController, ToastProvider } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const snapshot = await fixture();
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(TeamsScreen, {
          snapshot,
          bridge: mockBridge(snapshot),
          location: { kind: 'teams' },
          scene: 'groups',
          onNavigate,
          onRefresh: async () => {},
          onRefreshSnapshot: async () => snapshot,
          onError: (error: unknown) => {
            throw error;
          },
          onMutationError: async (error: unknown) => {
            throw error;
          },
          onLock: async () => true,
          agentLifecycle: { state: 'ready' },
          onRetryAgent: async () => {},
        }),
      }),
    }),
  );
  await ui.act(async () => {
    await Promise.resolve();
  });
  return rendered;
}

test('a Files row in an abnormal state carries a chip and is dimmed', async () => {
  await files();
  // Homelab's group reports setup incomplete in the fixture.
  const homelab = row('Homelab');
  assert.ok(homelab.className.split(' ').includes('off'));
  assert.equal(
    homelab.querySelector('.tail .chip')?.textContent,
    'Setup incomplete',
  );
  // The state is the chip, so the caption says only what the store is.
  assert.equal(homelab.querySelector('.name small')?.textContent, 'Share');

  const personal = row('Personal');
  assert.equal(personal.className, 'row');
  assert.equal(personal.querySelector('.tail .chip'), null);
  assert.equal(
    personal.querySelector('.name small')?.textContent,
    'Vault · foks.example.net',
  );
});

test('the Files rows open All items and the store they name', async () => {
  const journal: Location[] = [];
  await files((location) => journal.push(location));
  ui.fireEvent.click(row('All items'));
  assert.deepEqual(journal.at(-1), { kind: 'all' });
  ui.fireEvent.click(row('Engineering'));
  assert.deepEqual(journal.at(-1), { kind: 'store', ref: 'team:eng' });
});

test('a Teams row in an abnormal state carries the same chip, and opens group settings', async () => {
  const journal: Location[] = [];
  await teams((location) => journal.push(location));
  const homelab = row('Homelab');
  assert.ok(homelab.className.split(' ').includes('off'));
  assert.equal(
    homelab.querySelector('.tail .chip')?.textContent,
    'Setup incomplete',
  );
  // The caption is the server alone while the chip says what is wrong.
  assert.equal(
    homelab.querySelector('.name small')?.textContent,
    'foks.example.net',
  );

  const eng = row('Engineering');
  assert.equal(eng.className, 'row');
  assert.equal(eng.querySelector('.tail .chip'), null);
  await ui.act(async () => {
    ui.fireEvent.click(eng);
  });
  assert.deepEqual(journal.at(-1), {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'people',
  });
});

test('the Teams pane does not repeat the attention state under the list', async () => {
  const rendered = await teams();
  // The rows carry each store's state; the pane below creates and finds groups.
  assert.equal(rendered.queryByText('Needs attention'), null);
  assert.ok(rendered.getByText('Create a group'));
  assert.ok(rendered.getByText('Find groups'));
});
