/**
 * Verifies navigation guards for membership, server, and CLI handoff workflows.
 * Pending mutations block navigation, unsaved input prompts for confirmation,
 * and read-only loading does not block navigation.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge, GoProfileCandidate } from '../src/bridge';
import type { GuardVerdict, Location, LocationStore } from '../src/location';
import type { AgentSnapshot } from '../src/model';
import { installDom } from './lib/dom-harness';

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

function never<T>(): Promise<T> {
  return new Promise<T>(() => {});
}

const AWAY: Location = { kind: 'files' };

const CLI_CANDIDATE: GoProfileCandidate = {
  candidateId: 'candidate',
  username: 'cli-owner',
  hostId: '02' + 'ab'.repeat(32),
  userId: '01' + 'cd'.repeat(32),
  deviceId: '03' + 'ef'.repeat(32),
  role: 'owner',
  storageKind: 'macos-keychain',
  hidden: false,
  provisional: false,
  pairable: true,
  copyable: true,
};

async function harness() {
  const locations = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const guards = (await vite.ssrLoadModule(
    '/src/navigation-guard.tsx',
  )) as typeof import('../src/navigation-guard');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { ToastController, ToastProvider } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const group = FIXTURE.stores.find((store) => store.id === 'team:household');
  assert.ok(group, 'the fixture holds the Household group');

  const mount = (
    node: ReactNode,
    answer: boolean | null = null,
  ): {
    store: LocationStore;
    asked: GuardVerdict[];
    rendered: ReturnType<typeof ui.render>;
  } => {
    const store = new locations.LocationStore();
    const asked: GuardVerdict[] = [];
    store.setRefusalHandler(() => {});
    if (answer !== null)
      store.setPrompter(async (verdict) => {
        asked.push(verdict);
        return answer;
      });
    const rendered = ui.render(
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot,
        children: createElement(ToastProvider, {
          controller: new ToastController(),
          children: createElement(guards.NavigationGuardProvider, {
            store,
            children: node,
          }),
        }),
      }),
    );
    return { store, asked, rendered };
  };

  return {
    fixture: FIXTURE,
    group,
    mount,
    bridge: (overrides: Partial<Bridge> = {}): Bridge => ({
      ...mockBridge(FIXTURE),
      ...overrides,
    }),
    verdict: (store: LocationStore): GuardVerdict =>
      store.navigationVerdict({ kind: 'navigate', location: AWAY }),
    leave: async (store: LocationStore): Promise<void> => {
      await ui.act(async () => {
        store.navigate(AWAY);
        await Promise.resolve();
      });
    },
  };
}

/* -------------------------------------------------------- invitations -- */

async function invitationPanel(
  h: Awaited<ReturnType<typeof harness>>,
  bridge: Bridge,
  teamAlias?: string,
) {
  const { InvitationPanel } = (await vite.ssrLoadModule(
    '/src/components/invitation-panel.tsx',
  )) as typeof import('../src/components/invitation-panel');
  return h.mount(
    createElement(InvitationPanel, {
      bridge,
      profile: 'personal',
      account: 'personal',
      ...(teamAlias ? { teamAlias } : {}),
      onComplete: () => {},
    }),
    true,
  );
}

test('an untouched invitation form does not register a navigation guard', async () => {
  const h = await harness();
  const { store } = await invitationPanel(h, h.bridge());
  assert.equal(h.verdict(store), null);
});

test('unsaved invitation input prompts before navigation and clears on confirmation', async () => {
  const h = await harness();
  const { store, asked, rendered } = await invitationPanel(h, h.bridge());
  const field = rendered.getByLabelText('Invitation');
  await ui.act(async () => {
    ui.fireEvent.change(field, { target: { value: 'foks-invite-abc' } });
  });
  const question = h.verdict(store);
  assert.equal(question?.verdict, 'prompt');
  assert.equal(question.title, 'Discard invitation details?');
  assert.equal(
    question.body,
    'What is typed here has not been sent to the server.',
  );
  assert.equal(question.confirm, 'Discard');

  await h.leave(store);
  assert.equal(asked.length, 1);
  assert.deepEqual(store.getSnapshot().location, AWAY);
  assert.equal((field as HTMLInputElement).value, '');
});

test('a durable invitation creation allows navigation', async () => {
  const h = await harness();
  const { store, rendered } = await invitationPanel(
    h,
    h.bridge({ invitation: () => never() }),
    'household',
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Create invitation' }),
    );
    await Promise.resolve();
  });
  assert.equal(h.verdict(store), null);
  await h.leave(store);
  assert.deepEqual(store.getSnapshot().location, AWAY);
});

/* ------------------------------------------------------- group sheets -- */

async function groupSheet(
  h: Awaited<ReturnType<typeof harness>>,
  sheet: 'add' | 'create',
  onClose: () => void,
  bridge: Bridge = h.bridge(),
) {
  const { GroupSheet } = (await vite.ssrLoadModule(
    '/src/screens/groups-screen.tsx',
  )) as typeof import('../src/screens/groups-screen');
  return h.mount(
    createElement(GroupSheet, {
      snapshot: h.fixture,
      bridge,
      store: h.group,
      sheet,
      target: null,
      onClose,
      onSwitch: () => {},
      onApplied: async () => {},
      onMutationError: async () => {},
    }),
    true,
  );
}

test('loading member suggestions does not block navigation', async () => {
  const h = await harness();
  const { store } = await groupSheet(h, 'add', () => {});
  assert.equal(h.verdict(store), null);
});

test('unsaved member input prompts before navigation and closes on confirmation', async () => {
  const h = await harness();
  const closed: boolean[] = [];
  const { store, asked, rendered } = await groupSheet(h, 'add', () =>
    closed.push(true),
  );
  await ui.act(async () => {
    ui.fireEvent.change(rendered.getByLabelText('Username'), {
      target: { value: 'ada.lovelace' },
    });
  });
  const question = h.verdict(store);
  assert.equal(question?.verdict, 'prompt');
  assert.equal(question.title, 'Discard this member?');
  assert.equal(question.body, 'ada.lovelace has not been added to Household.');

  await h.leave(store);
  assert.equal(asked.length, 1);
  assert.deepEqual(store.getSnapshot().location, AWAY);
  assert.deepEqual(closed, [true]);
});

test('an unsaved team name prompts before navigation', async () => {
  const h = await harness();
  const { store, rendered } = await groupSheet(h, 'create', () => {});
  assert.equal(h.verdict(store), null);
  await ui.act(async () => {
    ui.fireEvent.change(rendered.getByLabelText('Name'), {
      target: { value: 'Robotics' },
    });
  });
  const question = h.verdict(store);
  assert.equal(question?.verdict, 'prompt');
  assert.equal(question.title, 'Discard this group?');
  assert.equal(question.body, 'Robotics has not been created.');
});

/* ------------------------------------------------------------ servers -- */

test('an active server check blocks navigation', async () => {
  const h = await harness();
  const { ServersSection } = (await vite.ssrLoadModule(
    '/src/screens/servers-screen.tsx',
  )) as typeof import('../src/screens/servers-screen');
  const unprobed: AgentSnapshot = {
    ...h.fixture,
    servers: h.fixture.servers.map((server) =>
      server.id === 'personal'
        ? { ...server, trust: { status: 'unprobed' as const } }
        : server,
    ),
  };
  const bridge = h.bridge({ checkServer: () => never() });
  const { store, rendered } = h.mount(
    createElement(ServersSection, {
      snapshot: unprobed,
      bridge,
      scene: 'settings',
      onNavigate: () => {},
      onRefresh: async () => {},
      onError: () => {},
      onMutationError: async () => {},
    }),
  );
  assert.equal(h.verdict(store), null);
  const check = await ui.waitFor(
    () => rendered.getAllByRole('button', { name: 'Check' })[0],
  );
  assert.ok(check);
  await ui.act(async () => {
    ui.fireEvent.click(check);
    await Promise.resolve();
  });
  assert.deepEqual(h.verdict(store), {
    verdict: 'refuse',
    reason: 'Wait for the server check to finish.',
  });
});

/* -------------------------------------------------------- CLI handoff -- */

test('scanning and adding a CLI server both allow navigation', async () => {
  const h = await harness();
  const { GoProfileConnectSheet } = (await vite.ssrLoadModule(
    '/src/screens/go-profile-connect.tsx',
  )) as typeof import('../src/screens/go-profile-connect');
  const bridge = h.bridge({
    discoverGoProfiles: async () => ({
      installed: true,
      candidates: [CLI_CANDIDATE],
    }),
    checkAndAddGoProfile: () => never(),
  });
  const { store, rendered } = h.mount(
    createElement(GoProfileConnectSheet, {
      bridge,
      onClose: () => {},
      onConnected: async () => {},
      onError: () => {},
    }),
  );
  // CLI profile discovery is read-only and therefore does not block navigation.
  assert.equal(h.verdict(store), null);
  const candidate = await ui.waitFor(() =>
    rendered.getByRole('radio', { name: /cli-owner/ }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(candidate);
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Use official FOKS server' }),
    );
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Add server' }));
    await Promise.resolve();
  });
  assert.equal(h.verdict(store), null);
});
