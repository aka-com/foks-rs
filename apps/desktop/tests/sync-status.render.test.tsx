/**
 * The topbar's refresh control: one square button, and the Refresh status
 * popover it opens on hover.
 *
 * The popover is portaled, so the pointer leaves the button's wrapper on its
 * way into the popover. The control tracks the pointer on both and defers the
 * close, and these tests hold that: crossing into the popover must not close
 * it, and leaving both must.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useRef } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
let OverlayProvider: typeof import('../kit/overlay-primitives').OverlayProvider;
let DesktopReconciliation: typeof import('../src/desktop-reconciliation').DesktopReconciliation;
let Topbar: typeof import('../src/shell/topbar').Topbar;
let FIXTURE: typeof import('../src/fixture').FIXTURE;

test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  ({ OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives'));
  ({ DesktopReconciliation } = (await vite.ssrLoadModule(
    '/src/desktop-reconciliation.ts',
  )) as typeof import('../src/desktop-reconciliation'));
  ({ Topbar } = (await vite.ssrLoadModule(
    '/src/shell/topbar.tsx',
  )) as typeof import('../src/shell/topbar'));
  ({ FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture'));
});

test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

/** Longer than the control's own close delay, so a close has had its chance. */
const AFTER_CLOSE_MS = 400;

function reconciliation(): InstanceType<typeof DesktopReconciliation> {
  return new DesktopReconciliation({
    snapshot: () => FIXTURE,
    nowSeconds: () => 20,
    profile: async () => {},
    registry: async () => {},
    discovery: async () => {},
    metadata: async () => {},
  });
}

function Harness({
  service,
  refreshing = false,
}: {
  service: InstanceType<typeof DesktopReconciliation>;
  refreshing?: boolean;
}) {
  const background = useRef<HTMLElement>(null);
  return createElement(
    'div',
    { ref: background },
    createElement(OverlayProvider, {
      backgroundRef: background,
      portalRoot: document.body,
      children: createElement(Topbar, {
        // These tests isolate refresh work; no chat provider is mounted.
        snapshot: {
          ...FIXTURE,
          stores: FIXTURE.stores.filter((store) => store.kind !== 'team'),
        },
        location: { kind: 'files' },
        onNavigate: () => {},
        collapsed: false,
        refreshing,
        onRefresh: () => {},
        syncService: service,
      }),
    }),
  );
}

/**
 * React derives enter and leave from the over and out events rather than from
 * the enter and leave ones, so the tests dispatch what it listens to.
 */
function enter(node: Element): void {
  ui.fireEvent.mouseOver(node);
}
function leave(node: Element): void {
  ui.fireEvent.mouseOut(node, { relatedTarget: document.body });
}

async function settle(): Promise<void> {
  await ui.act(async () => {
    await new Promise((resolve) => setTimeout(resolve, AFTER_CLOSE_MS));
  });
}

function mount(): Element {
  ui.render(createElement(Harness, { service: reconciliation() }));
  const wrap = document.querySelector('.global-refresh-wrap');
  assert.ok(wrap);
  return wrap;
}

test('the refresh control is one button, and pointing at it opens the status', async () => {
  const wrap = mount();
  // One button: refresh. The chevron that used to open the popover is gone,
  // and the popover is not up until the pointer arrives.
  assert.equal(wrap.querySelectorAll('button').length, 1);
  assert.ok(wrap.querySelector('button')?.className.includes('global-refresh'));
  assert.equal(document.querySelector('.sync-popover'), null);

  await ui.act(async () => {
    enter(wrap);
  });
  const popover = document.querySelector('.sync-popover');
  assert.ok(popover);
  assert.ok(popover.classList.contains('menu-portal'));
  // The open popover describes the button it belongs to.
  assert.equal(
    wrap.querySelector('button')?.getAttribute('aria-describedby'),
    popover.firstElementChild?.id,
  );
});

test('crossing from the button into the popover keeps it open', async () => {
  const wrap = mount();
  await ui.act(async () => {
    enter(wrap);
  });
  const popover = document.querySelector('.sync-popover');
  assert.ok(popover);

  // The pointer leaves the wrapper on its way into the portaled popover.
  await ui.act(async () => {
    leave(wrap);
    ui.fireEvent.pointerOver(popover);
  });
  await settle();
  assert.ok(document.querySelector('.sync-popover'));

  // Leaving the popover itself closes it.
  await ui.act(async () => {
    ui.fireEvent.pointerOut(popover, { relatedTarget: document.body });
  });
  await settle();
  assert.equal(document.querySelector('.sync-popover'), null);
});

test('leaving the button without reaching the popover closes it', async () => {
  const wrap = mount();
  await ui.act(async () => {
    enter(wrap);
  });
  assert.ok(document.querySelector('.sync-popover'));
  await ui.act(async () => {
    leave(wrap);
  });
  await settle();
  assert.equal(document.querySelector('.sync-popover'), null);
});

test('while refreshing, the spinner takes the icon’s place rather than sitting over it', async () => {
  const service = reconciliation();
  const activity = service.activities.begin('Loading catalog');
  const rendered = ui.render(
    createElement(Harness, { service, refreshing: true }),
  );
  const button = document.querySelector<HTMLButtonElement>('.global-refresh');
  assert.ok(button);
  assert.equal(button.getAttribute('aria-busy'), 'true');
  assert.equal(
    button.getAttribute('aria-label'),
    'Refreshing vaults, teams, chat, and devices',
  );
  assert.ok(button.querySelector('.spin'));
  assert.equal(button.querySelector('svg.ic'), null);
  assert.equal(document.querySelector('.sync-badge'), null);
  await ui.act(async () => activity.finish());
  rendered.rerender(createElement(Harness, { service, refreshing: false }));
  assert.equal(button.getAttribute('aria-busy'), null);
  assert.equal(button.querySelector('.spin'), null);
  assert.ok(button.querySelector('svg.ic'));
});

test('remaining work is visible beside healthy jobs and disappears when it settles', async () => {
  const service = reconciliation();
  service.update(FIXTURE);
  for (const observation of service.scheduler.observations())
    service.scheduler.reconciled(observation.key);
  const activity = service.activities.begin('Loading team rosters');
  const rendered = ui.render(createElement(Harness, { service }));
  const wrap = document.querySelector('.global-refresh-wrap')!;
  await ui.act(async () => enter(wrap));
  const button = wrap.querySelector('button')!;
  assert.equal(button.getAttribute('aria-busy'), 'true');
  assert.ok(
    ui.screen
      .getByLabelText('Other refresh activity')
      .textContent?.includes('Loading team rosters'),
  );
  const refreshNow = ui.screen.getByRole<HTMLButtonElement>('button', {
    name: 'Refresh now',
  });
  const copy = ui.screen.getByRole<HTMLButtonElement>('button', {
    name: 'Copy diagnostics',
  });
  const footerButtons = [...document.querySelectorAll('.sync-foot .btn')];
  assert.deepEqual(footerButtons, [copy, refreshNow]);
  assert.equal(refreshNow.textContent, '');
  assert.equal(refreshNow.title, 'Refresh now');
  assert.ok(refreshNow.querySelector('svg'));
  assert.equal(refreshNow.disabled, false);
  assert.equal(button.disabled, false);
  rendered.rerender(createElement(Harness, { service, refreshing: true }));
  assert.equal(refreshNow.disabled, true);
  assert.equal(button.disabled, true);
  await ui.act(async () => activity.finish());
  assert.equal(button.getAttribute('aria-busy'), null);
  assert.equal(ui.screen.queryByLabelText('Other refresh activity'), null);
  service.dispose();
});
