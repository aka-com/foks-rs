/** Rendering tests for the team-management action sheets. */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import type { GroupSheetKind } from '../src/screens/groups-screen';
import type { Party } from '../src/model';

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

async function sheet(kind: GroupSheetKind, target: string | null = null) {
  const { GroupSheet } = (await vite.ssrLoadModule(
    '/src/screens/groups-screen.tsx',
  )) as typeof import('../src/screens/groups-screen');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { partiesOf } = (await vite.ssrLoadModule(
    '/src/model/index.ts',
  )) as typeof import('../src/model');
  const { ToastController, ToastProvider } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const store = FIXTURE.stores.find((entry) => entry.id === 'team:eng');
  assert.ok(store?.kind === 'team');
  const party: Party | null = target
    ? (partiesOf(FIXTURE, store.id).find(
        (candidate) => candidate.username === target,
      ) ?? null)
    : null;
  if (target) assert.ok(party, `no party ${target}`);
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(GroupSheet, {
          snapshot: FIXTURE,
          bridge: mockBridge(FIXTURE),
          store,
          sheet: kind,
          target: party,
          onClose: () => {},
          onApplied: async () => {},
          onMutationError: async () => {},
        }),
      }),
    }),
  );
  await ui.act(async () => {});
  return rendered;
}

const radioTitles = (label: string): string[] =>
  [
    ...document.querySelectorAll(
      `[role="radiogroup"][aria-label="${label}"] [role="radio"] .t b`,
    ),
  ].map((node) => node.textContent ?? '');

test('lowering a role states the present role and offers only the roles below it', async () => {
  const r = await sheet('demote', 'priya.n');
  assert.ok(r.getByRole('heading', { name: 'Lower priya.n’s role' }));
  // The current role is displayed as context, not as a disabled option.
  const fact = document.querySelector('.sheet .inset .fr');
  assert.ok(fact);
  assert.equal(fact.querySelector('.t b')?.textContent, 'priya.n');
  assert.equal(fact.querySelector('.a .chip')?.textContent, 'Admin');
  assert.equal(document.querySelector('.sheet [role="radio"].off'), null);
  // Member is the only lower role for an Admin.
  assert.deepEqual(radioTitles('New role'), ['Member']);
  assert.equal(
    (r.getByRole('button', { name: 'Change role' }) as HTMLButtonElement)
      .disabled,
    false,
  );
  assert.match(
    document.querySelector('.sheet .fn')?.textContent ?? '',
    /Roles can only be lowered here/,
  );
});

test('creating a team opens on its kind, with nothing behind a disclosure', async () => {
  const r = await sheet('create');
  assert.ok(r.getByRole('heading', { name: 'Create a team' }));
  const groups = [
    ...document.querySelectorAll('.sheet [role="radiogroup"]'),
  ].map((node) => node.getAttribute('aria-label'));
  assert.deepEqual(groups, ['Kind', 'Server and account']);
  assert.deepEqual(radioTitles('Kind'), ['Named team', 'Ad-hoc share']);
  assert.equal(document.querySelector('.sheet details'), null);
  assert.ok(r.getByRole('button', { name: 'Create team' }));
  ui.fireEvent.click(r.getByRole('radio', { name: /Ad-hoc share/ }));
  assert.ok(r.getByRole('button', { name: 'Create share' }));
  ui.fireEvent.change(r.getByLabelText('Name'), {
    target: { value: 'Q3 launch files' },
  });
  assert.match(
    document.querySelector('.sheet .fn')?.textContent ?? '',
    /Only this device sees the name\. Stored as q3-launch-files/,
  );
});
