import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { AgentSnapshot, TeamStore } from '../src/model';
import type { Location } from '../src/location';
import type { Bridge } from '../src/bridge';
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
async function mount(
  transform: (snapshot: AgentSnapshot) => AgentSnapshot = (snapshot) =>
    snapshot,
  overrides: Partial<Bridge> = {},
) {
  const { TeamInfoPanel } = await import('../src/screens/team-info');
  const { ChatInboxProvider } = await import('../src/chat/inbox-provider');
  const { FIXTURE } = await import('../src/mock/fixture');
  const { mockBridge } = await import('../src/mock/mock-bridge');
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
    ...overrides,
  };
  const panel = (shown: AgentSnapshot) =>
    createElement(ChatInboxProvider, {
      snapshot: shown,
      bridge,
      enabled: false,
      children: createElement(TeamInfoPanel, {
        snapshot: shown,
        store: shown.stores.find((entry) => entry.id === store.id) as TeamStore,
        bridge,
        requestCount: 2,
        onNavigate: (location) => navigations.push(location),
        onClose: (restore) => closed.push(restore),
        onError: (error) => {
          throw error;
        },
      }),
    });
  let rendered!: ReturnType<typeof ui.render>;
  await ui.act(async () => {
    rendered = ui.render(panel(snapshot));
  });
  const rerender = async (shown: AgentSnapshot) =>
    ui.act(async () => rendered.rerender(panel(shown)));
  return { snapshot, store, navigations, closed, copies, rerender };
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

test('the Files section reads storage used once on open and again after the files change', async () => {
  const reads: string[] = [];
  const mounted = await mount(undefined, {
    storeUsage: async (storeId) => {
      reads.push(storeId);
      return { files: 1_204, bytes: 2.3 * 2 ** 30 };
    },
  });
  assert.ok(
    await ui.screen.findByText(
      'About 2.3 GiB stored (encrypted) · 1,204 files',
    ),
  );
  assert.deepEqual(reads, ['team:eng']);
  // A republished catalog with the same files does not read again.
  await mounted.rerender({ ...mounted.snapshot });
  assert.deepEqual(reads, ['team:eng']);
  const changed = {
    ...mounted.snapshot,
    items: mounted.snapshot.items.map((item) =>
      item.store === 'team:eng' && item.kind !== 'Folder'
        ? { ...item, version: item.version + 1 }
        : item,
    ),
  };
  await mounted.rerender(changed);
  await ui.waitFor(() => assert.deepEqual(reads, ['team:eng', 'team:eng']));
});

test('a failed usage read is stated in place without an error notice', async () => {
  await mount(undefined, {
    storeUsage: async () => {
      throw { code: 'profile-busy', message: 'Busy.', retryable: true };
    },
  });
  assert.ok(await ui.screen.findByText('Storage used unavailable.'));
});

test('an incomplete team does not read storage used', async () => {
  const reads: string[] = [];
  await mount(
    (snapshot) => ({
      ...snapshot,
      stores: snapshot.stores.map((store) =>
        store.id === 'team:eng' ? { ...store, active: false } : store,
      ),
    }),
    {
      storeUsage: async (storeId) => {
        reads.push(storeId);
        return { files: 0, bytes: 0 };
      },
    },
  );
  assert.equal(ui.screen.queryByText(/stored \(encrypted\)/), null);
  assert.deepEqual(reads, []);
});
