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

test('announces Syncing while refreshing and Synced when finished', async () => {
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
    'Syncing… — refreshing vaults, teams, chat and devices',
  );
  // Preserve the text-only refresh treatment while exposing its busy state.
  assert.ok(button.classList.contains('busy'));
  assert.equal(button.querySelector('svg.ic'), null);
  assert.equal(button.querySelector('.spin'), null);
  assert.equal(button.querySelector('.dot'), null);
  assert.equal(button.querySelector('.t')?.textContent, 'Syncing…');
  await ui.act(async () => activity.finish());
  rendered.rerender(createElement(Harness, { service, refreshing: false }));
  assert.equal(button.getAttribute('aria-busy'), null);
  assert.equal(button.classList.contains('busy'), false);
  assert.equal(button.querySelector('svg.ic'), null);
  // When idle, displays a green status dot alongside the "Synced" label.
  assert.ok(button.querySelector('.dot.ok'));
  assert.equal(button.querySelector('.t')?.textContent, 'Synced');
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
  assert.equal(refreshNow.textContent, 'Refresh now');
  assert.equal(refreshNow.title, 'Refresh now');
  assert.ok(refreshNow.querySelector('svg'));
  assert.equal(refreshNow.disabled, true);
  assert.equal(button.disabled, false);
  assert.equal(button.getAttribute('aria-disabled'), 'true');
  rendered.rerender(createElement(Harness, { service, refreshing: true }));
  assert.equal(refreshNow.disabled, true);
  assert.equal(button.getAttribute('aria-disabled'), 'true');
  rendered.rerender(createElement(Harness, { service, refreshing: false }));
  await ui.act(async () => activity.finish());
  assert.equal(button.getAttribute('aria-busy'), null);
  assert.equal(ui.screen.queryByLabelText('Other refresh activity'), null);
  service.dispose();
});

test('ArrowDown moves the focus into the popover, and Escape brings it back', async () => {
  const wrap = mount();
  const button = wrap.querySelector<HTMLButtonElement>('button');
  assert.ok(button);
  await ui.act(async () => {
    button.focus();
  });
  assert.ok(document.querySelector('.sync-popover'));

  await ui.act(async () => {
    ui.fireEvent.keyDown(button, { key: 'ArrowDown' });
  });
  const popover = document.querySelector('.sync-popover');
  assert.ok(popover);
  const active = document.activeElement;
  assert.ok(active instanceof HTMLElement);
  assert.ok(popover.contains(active), 'focus lands inside the popover');
  // The button's blur and the control's focus are one move: the popover stays.
  await settle();
  assert.ok(document.querySelector('.sync-popover'));

  await ui.act(async () => {
    ui.fireEvent.keyDown(document, { key: 'Escape' });
  });
  await settle();
  assert.equal(document.querySelector('.sync-popover'), null);
  assert.equal(document.activeElement, button);
});

test('the refresh control reads out the state it shows and stops while it runs', () => {
  ui.render(
    createElement(Harness, { service: reconciliation(), refreshing: true }),
  );
  const button = document.querySelector<HTMLButtonElement>('.global-refresh');
  assert.ok(button);
  // The accessible name carries the visible word rather than contradicting it.
  assert.match(button.getAttribute('aria-label') ?? '', /^Syncing… — /);
  assert.equal(button.textContent?.includes('Syncing…'), true);
  // A refresh already in flight is not started again from here.
  assert.equal(button.disabled, false);
  assert.equal(button.getAttribute('aria-disabled'), 'true');
  assert.equal(button.getAttribute('aria-busy'), 'true');
});

test('Chat with no team that has chat offers a team and disables the field', () => {
  const navigations: unknown[] = [];
  const background = { current: null } as { current: HTMLElement | null };
  ui.render(
    createElement(
      'div',
      null,
      createElement(OverlayProvider, {
        backgroundRef: background,
        portalRoot: document.body,
        children: createElement(Topbar, {
          snapshot: FIXTURE,
          location: { kind: 'chat' },
          onNavigate: (location) => navigations.push(location),
          collapsed: false,
          query: '',
          onQuery: () => {},
          onNewChat: () => {},
          chatTeamCount: 0,
        }),
      }),
    ),
  );
  const field = document.querySelector<HTMLInputElement>('.topsearch input');
  assert.ok(field);
  assert.equal(field.disabled, true);
  assert.equal(ui.screen.queryByRole('button', { name: 'New chat' }), null);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'New team' }));
  assert.deepEqual(navigations, [{ kind: 'teams', open: 'create' }]);
});

test('Chat with teams keeps New chat and a live field', () => {
  const background = { current: null } as { current: HTMLElement | null };
  ui.render(
    createElement(
      'div',
      null,
      createElement(OverlayProvider, {
        backgroundRef: background,
        portalRoot: document.body,
        children: createElement(Topbar, {
          snapshot: FIXTURE,
          location: { kind: 'chat' },
          onNavigate: () => {},
          collapsed: false,
          query: '',
          onQuery: () => {},
          onNewChat: () => {},
          chatTeamCount: 2,
        }),
      }),
    ),
  );
  const field = document.querySelector<HTMLInputElement>('.topsearch input');
  assert.ok(field);
  assert.equal(field.disabled, false);
  assert.ok(ui.screen.getByRole('button', { name: 'New chat' }));
});

for (const location of [
  { kind: 'chat', ref: 'team:eng', channel: 'general' },
  { kind: 'settings', section: 'account', profile: 'acme' },
] as const) {
  test(`the intermediate ${location.kind} breadcrumb navigates to its parent`, async () => {
    const navigations: unknown[] = [];
    const { ChatInboxProvider } = (await vite.ssrLoadModule(
      '/src/chat/inbox-provider.tsx',
    )) as typeof import('../src/chat/inbox-provider');
    const { mockBridge } = (await vite.ssrLoadModule(
      '/src/mock-bridge.ts',
    )) as typeof import('../src/mock-bridge');
    const snapshot = {
      ...FIXTURE,
      servers: FIXTURE.servers.map((server) => ({
        ...server,
        services: { ...server.services, chat: true },
      })),
    };
    const bridge = mockBridge(snapshot);
    let target: import('../src/location').Location = location;
    if (location.kind === 'chat') {
      const reply = await bridge.chat(location.ref, {
        action: 'sync-inbox',
        blocked_channels: [],
      });
      assert.equal(reply.result.kind, 'inbox');
      if (reply.result.kind !== 'inbox') return;
      target = {
        ...location,
        channel: reply.result.conversations[0].channel.id,
      };
    }
    ui.render(
      createElement(ChatInboxProvider, {
        snapshot,
        bridge,
        children: createElement(Topbar, {
          snapshot,
          location: target,
          collapsed: false,
          onNavigate: (next) => navigations.push(next),
        }),
      }),
    );
    await ui.waitFor(() =>
      assert.equal(document.querySelectorAll('.crumbs button').length, 3),
    );
    const crumbs =
      document.querySelectorAll<HTMLButtonElement>('.crumbs button');
    assert.equal(crumbs.length, 3);
    ui.fireEvent.click(crumbs[1]);
    assert.deepEqual(navigations, [
      location.kind === 'chat'
        ? { kind: 'chat', ref: 'team:eng' }
        : { kind: 'settings', section: 'account' },
    ]);
  });
}

test('an unavailable team retains its settings action in the takeover band', async () => {
  const { StoreAccessTakeover } = (await vite.ssrLoadModule(
    '/src/screens/store-access.tsx',
  )) as typeof import('../src/screens/store-access');
  const team = FIXTURE.stores.find((store) => store.kind === 'team');
  assert.ok(team && team.kind === 'team');
  let opened = 0;
  ui.render(
    createElement(StoreAccessTakeover, {
      snapshot: FIXTURE,
      store: { ...team, active: false },
      variant: 'band',
      onOpenServer: () => {},
      onFinishSetup: () => {},
      headerAction: createElement(
        'button',
        { onClick: () => opened++ },
        'Team settings',
      ),
    }),
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Team settings' }));
  assert.equal(opened, 1);
});
