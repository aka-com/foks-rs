import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { AgentSnapshot, TeamStore } from '../src/model';
import type { Location } from '../src/location';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div>',
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
async function mount(
  transform: (snapshot: AgentSnapshot) => AgentSnapshot = (snapshot) =>
    snapshot,
) {
  const { TeamInfoPanel } = (await vite.ssrLoadModule(
    '/src/screens/team-info.tsx',
  )) as typeof import('../src/screens/team-info');
  const { ChatInboxProvider } = (await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
  )) as typeof import('../src/chat/inbox-provider');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const snapshot = transform(FIXTURE);
  const store = snapshot.stores.find(
    (store) => store.id === 'team:eng',
  ) as TeamStore;
  const navigations: Location[] = [];
  const closed: (boolean | undefined)[] = [];
  const copies: string[] = [];
  const bridge = {
    ...mockBridge(snapshot),
    copyText: async (text: string) => {
      copies.push(text);
      return { ok: true as const };
    },
  };
  await ui.act(async () => {
    ui.render(
      createElement(ChatInboxProvider, {
        snapshot,
        bridge,
        enabled: false,
        children: createElement(TeamInfoPanel, {
          snapshot,
          store,
          bridge,
          requestCount: 2,
          onNavigate: (location) => navigations.push(location),
          onClose: (restore) => closed.push(restore),
          onError: (error) => {
            throw error;
          },
        }),
      }),
    );
  });
  return { snapshot, store, navigations, closed, copies };
}

test('Team info consolidates identity, membership, files and requests with existing destinations', async () => {
  const mounted = await mount();
  const panel = ui.screen.getByRole('complementary', { name: 'Team info' });
  assert.ok(document.activeElement === panel, 'Panel receives focus');
  for (const name of ['Channels', 'Files', 'Requests', 'Team ID'])
    assert.ok(ui.within(panel).getByRole('heading', { name }));
  assert.ok(ui.within(panel).getByRole('heading', { name: /^Members ·/ }));
  assert.ok(ui.screen.getByText('2 pending requests'));
  assert.ok(ui.screen.getByText('Machine'));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Manage members' }));
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Open team files' }),
  );
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Review requests' }),
  );
  assert.deepEqual(mounted.navigations, [
    { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
    { kind: 'store', ref: 'team:eng' },
    { kind: 'group-settings', ref: 'team:eng', tab: 'requests' },
  ]);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Copy team ID' }));
  assert.deepEqual(mounted.copies, [mounted.store.team_id_hex]);
});

test('dismissal distinguishes keyboard focus restoration from outside pointer activity', async () => {
  const mounted = await mount();
  const panel = ui.screen.getByRole('complementary', { name: 'Team info' });
  ui.fireEvent.pointerDown(panel, { button: 0 });
  assert.deepEqual(mounted.closed, []);
  ui.fireEvent.keyDown(panel, { key: 'Escape' });
  ui.fireEvent.pointerDown(document.body, { button: 0 });
  assert.deepEqual(mounted.closed, [undefined, false]);
});

test('ordinary members see no request administration', async () => {
  await mount((snapshot) => ({
    ...snapshot,
    parties: snapshot.parties.map((party) =>
      party.store === 'team:eng' && party.label === 'you'
        ? { ...party, destination_role: 'member' }
        : party,
    ),
  }));
  assert.equal(ui.screen.queryByRole('heading', { name: 'Requests' }), null);
  assert.equal(
    ui.screen.queryByRole('button', { name: 'Review requests' }),
    null,
  );
});

test('an incomplete team shows its identity and status without cached member or channel details', async () => {
  await mount((snapshot) => ({
    ...snapshot,
    stores: snapshot.stores.map((store) =>
      store.id === 'team:eng' ? { ...store, active: false } : store,
    ),
  }));
  assert.ok(ui.screen.getByRole('heading', { name: 'Engineering' }));
  assert.equal(ui.screen.queryByRole('heading', { name: /^Members/ }), null);
  assert.equal(ui.screen.queryByRole('heading', { name: /^Channels/ }), null);
  assert.ok(ui.screen.getByRole('status'));
});

test('roster read failures withhold counts and cached member rows', async () => {
  await mount((snapshot) => ({
    ...snapshot,
    groupDetailFailures: [
      {
        store: 'team:eng',
        source: 'roster',
        code: 'unavailable',
        message: 'Roster temporarily unavailable.',
        retryable: true,
      },
    ],
  }));
  assert.ok(ui.screen.getByRole('heading', { name: 'Members' }));
  assert.ok(ui.screen.getByRole('alert'));
  assert.ok(!ui.screen.queryByText('sam.ortiz'));
  assert.ok(!ui.screen.queryByRole('heading', { name: 'Requests' }));
});

test('team info waits for roster enrichment after the item catalog is complete', async () => {
  await mount((snapshot) => ({
    ...snapshot,
    parties: [],
    federation: [],
    groupDetailInventory: [
      { store: 'team:eng', roster: 'loading', federation: 'loading' },
    ],
    catalogFreshness: {
      profiles: {
        local: { refreshing: false, lastAttemptAt: 1, lastSuccessAt: 1 },
      },
      stores: {},
    },
  }));
  assert.ok(ui.screen.getByText('Loading members…'));
  assert.ok(ui.screen.getByText('Loading owner…'));
  assert.equal(ui.screen.queryByText('No members yet.'), null);
  assert.equal(ui.screen.queryByText('No owner designated'), null);
});
