/**
 * The Activity page: membership requests from every team this Mac manages,
 * and the notices the snapshot holds. Requests are read and approved through
 * the same hook as the team page's Requests tab, so an approval made here is
 * gone there, sent as the same command.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';
import type { Bridge } from '../src/bridge';
import type { InvitationAction } from '../src/invitation-contract';
import type { Location } from '../src/location';
import type { MetadataRepository } from '../src/metadata-repository';
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

async function fixture(): Promise<AgentSnapshot> {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  return FIXTURE;
}

/** A fixture bridge that records every invitation action it is sent. */
async function recordingBridge(snapshot: AgentSnapshot): Promise<{
  bridge: Bridge;
  sent: { profile: string; action: InvitationAction }[];
}> {
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const base = mockBridge(snapshot);
  const sent: { profile: string; action: InvitationAction }[] = [];
  return {
    sent,
    bridge: {
      ...base,
      invitation: (profile, account, action, secret) => {
        sent.push({ profile, action });
        return base.invitation(profile, account, action, secret);
      },
    },
  };
}

async function providers(child: ReturnType<typeof createElement>) {
  const { ToastController, ToastProvider } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  return createElement(OverlayProvider, {
    backgroundRef: { current: null },
    portalRoot,
    children: createElement(ToastProvider, {
      controller: new ToastController(),
      children: child,
    }),
  });
}

async function activity(
  snapshot: AgentSnapshot,
  bridge: Bridge,
  navigate: (location: Location) => void = () => {},
  repository?: MetadataRepository,
) {
  const { ActivityScreen } = (await vite.ssrLoadModule(
    '/src/screens/activity-screen.tsx',
  )) as typeof import('../src/screens/activity-screen');
  const { MetadataRepositoryContext } = (await vite.ssrLoadModule(
    '/src/query-hooks.ts',
  )) as typeof import('../src/query-hooks');
  const screen = createElement(ActivityScreen, {
    snapshot,
    bridge,
    query: '',
    onNavigate: navigate,
    onApplied: async () => {},
    onError: (error: unknown) => {
      throw error;
    },
    onMutationError: async () => {},
  });
  const rendered = ui.render(
    await providers(
      repository
        ? createElement(MetadataRepositoryContext.Provider, {
            value: repository,
            children: screen,
          })
        : screen,
    ),
  );
  await ui.act(async () => {});
  return rendered;
}

function rowTitles(): string[] {
  return [...document.querySelectorAll('[role="option"] .name b')].map(
    (node) => node.textContent ?? '',
  );
}

test('Activity lists requests, then the notices the snapshot holds', async () => {
  const snapshot = await fixture();
  const { bridge } = await recordingBridge(snapshot);
  await activity(snapshot, bridge);
  await ui.waitFor(() =>
    assert.equal(
      document.querySelectorAll('[role="option"]').length >= 4,
      true,
    ),
  );
  const sections = [...document.querySelectorAll('.gsec')].map(
    (node) => node.textContent,
  );
  assert.deepEqual(sections, ['Waiting for you', 'Needs attention']);
  const titles = rowTitles();
  // Two fixture requests on each manageable team with a pending count.
  assert.equal(titles.filter((title) => title === 'fixture-joiner').length, 2);
  // The unfinished team, the inactive federated admission (named by the
  // display name of the team this device holds, not its alias) and the
  // unverified server.
  assert.ok(titles.includes('Homelab is inactive'));
  assert.ok(titles.includes('Homelab cannot access Engineering'));
  assert.ok(titles.includes('foks.partner.dev is not verified'));
  // The first entry is selected and its details are drawn.
  assert.equal(
    document.querySelector('[role="option"][aria-selected="true"] .name b')
      ?.textContent,
    'fixture-joiner',
  );
  assert.ok(ui.screen.getByRole('button', { name: 'Approve' }));
});

test('approving in Activity is the Requests tab approval, on the same rows', async () => {
  const snapshot = await fixture();
  const { bridge, sent } = await recordingBridge(snapshot);
  const rendered = await activity(snapshot, bridge);
  await ui.waitFor(() =>
    assert.ok(ui.screen.getByRole('button', { name: 'Approve' })),
  );
  const selected = document.querySelector(
    '[role="option"][aria-selected="true"] small',
  )?.textContent;
  assert.match(selected ?? '', /^Wants to join Engineering/);
  await ui.act(async () => {
    ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Approve' }));
  });
  await ui.waitFor(() =>
    assert.ok(
      sent.some(
        (entry) =>
          entry.profile === 'acme' &&
          entry.action.action === 'approve' &&
          entry.action.team_alias === 'engineering',
      ),
    ),
  );
  const approve = sent.find((entry) => entry.action.action === 'approve');
  assert.ok(approve && approve.action.action === 'approve');
  assert.deepEqual(approve.action.role, { member: { visibility: 0 } });
  rendered.unmount();

  // The team page reads the same inbox: the approved request is gone there.
  const { GroupSettingsScreen } = (await vite.ssrLoadModule(
    '/src/screens/groups-screen.tsx',
  )) as typeof import('../src/screens/groups-screen');
  ui.render(
    await providers(
      createElement(GroupSettingsScreen, {
        snapshot,
        bridge,
        location: { kind: 'group-settings', ref: 'team:eng', tab: 'requests' },
        onNavigate: () => {},
        onApplied: async () => {},
        onError: (error: unknown) => {
          throw error;
        },
        onMutationError: async () => {},
      }),
    ),
  );
  await ui.waitFor(() =>
    assert.equal(document.querySelectorAll('.requests .request-row').length, 1),
  );
  assert.equal(
    document.querySelector('.requests .request-row .t b span')?.textContent,
    'fixture-second-joiner',
  );
});

test('a server notice opens that server, and the list follows the arrow keys', async () => {
  const snapshot = await fixture();
  const { bridge } = await recordingBridge(snapshot);
  const journal: Location[] = [];
  await activity(snapshot, bridge, (location) => journal.push(location));
  await ui.waitFor(() => assert.ok(rowTitles().length >= 4));
  const options = [
    ...document.querySelectorAll<HTMLButtonElement>('[role="option"]'),
  ];
  const server = options.find(
    (node) =>
      node.querySelector('.name b')?.textContent ===
      'foks.partner.dev is not verified',
  );
  assert.ok(server);
  ui.fireEvent.click(server);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Open Server' }));
  assert.deepEqual(journal.at(-1), {
    kind: 'settings',
    section: 'account',
    profile: 'partner',
  });
  // Up moves the selection, and focus with it, to the entry above.
  server.focus();
  await ui.act(async () => {
    ui.fireEvent.keyDown(server, { key: 'ArrowUp' });
  });
  const now = document.querySelector<HTMLElement>(
    '[role="option"][aria-selected="true"]',
  );
  assert.equal(
    now?.querySelector('.name b')?.textContent,
    'Homelab cannot access Engineering',
  );
  assert.equal(document.activeElement, now);
});

test('the Activity count adds requests to the snapshot notices', async () => {
  const snapshot = await fixture();
  const { activityBadge, activityNotices } = (await vite.ssrLoadModule(
    '/src/activity.ts',
  )) as typeof import('../src/activity');
  assert.equal(activityNotices(snapshot).length, 3);
  assert.deepEqual(activityBadge(snapshot, new Map()), {
    label: '3',
    description: '3 notices',
  });
  assert.deepEqual(activityBadge(snapshot, new Map([['team:eng', 2]])), {
    label: '5',
    description: '2 requests and 3 notices',
  });
  assert.deepEqual(activityBadge(snapshot, new Map([['team:eng', '2+']])), {
    label: '5+',
    description: 'At least 2 requests and 3 notices',
  });
  // A team with cached unfinished work is one more notice.
  assert.deepEqual(
    activityBadge(snapshot, new Map(), new Map([['team:eng', 2]])),
    { label: '4', description: '4 notices' },
  );
});

test('a team’s unfinished changes are listed from the cached journal, never read by Activity', async () => {
  const snapshot = await fixture();
  const { bridge: base } = await recordingBridge(snapshot);
  const { MetadataRepository } = (await vite.ssrLoadModule(
    '/src/metadata-repository.ts',
  )) as typeof import('../src/metadata-repository');
  const { pendingOperationsQuery } = (await vite.ssrLoadModule(
    '/src/operation-queries.ts',
  )) as typeof import('../src/operation-queries');
  const repository = new MetadataRepository();
  let journalReads = 0;
  const bridge: Bridge = {
    ...base,
    listPendingOperations: async () => {
      journalReads += 1;
      return [
        {
          kind: 'team-member-addition',
          alias: 'engineering',
          target: 'jules.park',
        },
      ];
    },
  };
  await activity(snapshot, bridge, () => {}, repository);
  await ui.waitFor(() =>
    assert.ok(rowTitles().includes('Homelab is inactive')),
  );
  // Nothing has read Engineering's profile journal, so Activity neither
  // lists it nor reads it.
  assert.equal(
    rowTitles().includes('Engineering has unfinished changes'),
    false,
  );
  assert.equal(journalReads, 0);

  // The team page reads it (here, directly through the shared query); the
  // entry follows without a read of Activity's own.
  const eng = snapshot.stores.find((store) => store.id === 'team:eng');
  assert.ok(eng);
  await ui.act(async () => {
    await pendingOperationsQuery(repository, bridge, eng.server).load();
  });
  assert.equal(journalReads, 1);
  await ui.waitFor(() =>
    assert.ok(rowTitles().includes('Engineering has unfinished changes')),
  );
  const row = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find(
    (option) =>
      option.querySelector('.name b')?.textContent ===
      'Engineering has unfinished changes',
  );
  assert.ok(row);
  assert.equal(
    row.querySelector('.name small')?.textContent,
    '1 incomplete membership change',
  );
  await ui.act(async () => ui.fireEvent.click(row));
  // Its details state what the team page's band states, and Review opens
  // the same sheet over the team's own operations.
  await ui.waitFor(() =>
    assert.ok(
      ui.screen.getByText(
        '1 incomplete membership change. It blocks other actions.',
      ),
    ),
  );
  await ui.act(async () =>
    ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Review…' })),
  );
  const dialog = ui.screen.getByRole('dialog');
  assert.ok(
    ui
      .within(dialog)
      .getByRole('heading', { name: 'Unfinished activity · Engineering' }),
  );
  assert.ok(ui.within(dialog).getByText('Adding jules.park'));
});
