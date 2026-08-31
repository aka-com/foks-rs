/**
 * One render per deep link — the states `README.md`'s table names, and the
 * ones the Playwright walk loads.
 *
 * Every string asserted about a person, a roster or a count is **computed
 * from the fixture here and compared with what the screen drew**, never typed
 * in as an expectation of its own. Where the plan names a golden ("3 people",
 * "5 people · 1 group") the derivation is checked against that word too, so a
 * model that quietly changed cannot make both sides agree on the wrong
 * answer; `model.test.ts` is where those goldens live.
 *
 * The shell is loaded through Vite's `ssrLoadModule` because it imports
 * `/kit/*` and `/src/*` — aliases only the bundler resolves. The fixture and
 * the model are plain relative modules, so they are imported directly and are
 * the same values the app is rendering.
 */

import assert from 'node:assert/strict';
import test, { after, afterEach } from 'node:test';
import { createElement, StrictMode } from 'react';
import type { ComponentType } from 'react';
import { createServer } from 'vite';
import type { ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';

const dom = installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});

import { FIXTURE } from '../src/fixture';
import {
  applyLease,
  catalog,
  itemKey,
  notesNow,
  partiesOf,
  peopleGroups,
  peopleLabel,
  readersOf,
} from '../src/model';
import type { Item, World } from '../src/model';
import { FOKS_ICONS } from '../src/icons';
import { mockBridge } from '../src/mock-bridge';
import type {
  AccountDevice,
  AppLockState,
  Bridge,
  CreateTextRequest,
} from '../src/bridge';
import { roleDto } from '../src/bridge';
import {
  decodeFirstRunCheckpoint,
  encodeFirstRunCheckpoint,
  initialFirstRun,
} from '../src/first-run-state';
import type { AppProps } from '../src/app-root';

type TestingLibrary = typeof import('@testing-library/react');
let testingLibrary: TestingLibrary;
let vite: ViteDevServer;
let App: ComponentType<AppProps>;

test.before(async () => {
  testingLibrary = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  App = (
    (await vite.ssrLoadModule('/src/app-root.tsx')) as unknown as {
      App: ComponentType<AppProps>;
    }
  ).App;
});

after(async () => {
  testingLibrary.cleanup();
  await vite.close();
  dom.window.close();
});

afterEach(async () => {
  await flushPhase6Loads();
  testingLibrary.cleanup();
});

/** Render the shell at a deep link, the way a reload of that address would. */
function at(search: string, props: AppProps = {}): void {
  window.history.replaceState(null, '', `/${search}`);
  const world = props.world ?? FIXTURE;
  testingLibrary.render(
    createElement(App, {
      world,
      bridge: props.bridge ?? mockBridge(world),
      ...props,
    }),
  );
}

async function flushPhase6Loads(): Promise<void> {
  await testingLibrary.act(async () => {
    for (let pass = 0; pass < 5; pass += 1) {
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
  });
}

function nativeAvailableBridge(base: Bridge): Bridge {
  return {
    ...base,
    native: true,
    firstRunFixture: undefined,
    fixtureWorld: undefined,
    describeServerStatus: async (profile: string) => {
      const status = await base.describeServerStatus(profile);
      return {
        ...status,
        leaseExpiresAt: status.host ? Math.floor(Date.now() / 1000) + 86_400 : null,
      };
    },
  };
}

const text = (selector: string): string =>
  document.querySelector(selector)?.textContent?.trim() ?? '';

const all = (selector: string): string[] =>
  [...document.querySelectorAll(selector)].map(
    (node) => node.textContent?.trim() ?? '',
  );

/** The row whose name cell starts with this name. */
function row(name: string): HTMLElement {
  const found = [...document.querySelectorAll<HTMLElement>('.body .row')].find(
    (candidate) =>
      candidate.querySelector('.name .tt')?.textContent?.startsWith(name),
  );
  assert.ok(found, `${name} is listed`);
  return found;
}

/** What `readersOf` answers for one item, as the row's chip words it. */
function readerChip(world: World, key: string): string {
  const item = world.items.find((candidate) => itemKey(candidate) === key);
  assert.ok(item, `${key} is in the fixture`);
  const readers = readersOf(world, item);
  assert.ok(readers, `${key} is in a group store`);
  return peopleLabel(readers.length);
}

/* ------------------------------------------------------------------- all -- */

test('?state=all lists the whole catalog, sorted by name', () => {
  at('?state=all');
  assert.equal(text('.loc h1'), 'All items');
  assert.equal(
    document.querySelectorAll('.body .row').length,
    catalog(FIXTURE).length,
  );
  assert.equal(text('.body .hdr'), 'Name ↓ServerReadable byVersion');
  const github = row('github.com');
  const githubItem = FIXTURE.items.find(
    (item) => itemKey(item) === 'acct:personal|/logins/github.com',
  );
  assert.ok(githubItem);
  assert.equal(github.querySelector('.server')?.textContent, 'foks.example.net');
  assert.equal(github.querySelector('.chip')?.textContent, '1');
  assert.equal(github.querySelector(':scope > .n:last-child')?.textContent, String(githubItem.version));
  assert.equal(github.querySelector('button'), null, 'rows select without inline actions');
  assert.equal(
    document.querySelector('.search input')?.getAttribute('placeholder'),
    'Search all items',
  );
});

test('App with an injected world but no bridge fails closed instead of hanging', async () => {
  window.history.replaceState(null, '', '/?state=all');
  testingLibrary.render(createElement(App, { world: FIXTURE }));
  await testingLibrary.waitFor(() =>
    assert.match(text('[role="alert"]'), /must include the bridge/),
  );
});

test('a native app lock unlocks before the agent or catalog can be read', async () => {
  window.history.replaceState(null, '', '/?state=all');
  const base = mockBridge(FIXTURE);
  const calls: string[] = [];
  let locked = true;
  let finishUnlock: ((state: AppLockState) => void) | undefined;
  const bridge: Bridge = {
    ...nativeAvailableBridge(base),
    appLockState: async () => {
      calls.push('lock-state');
      return { locked, available: true, mechanism: 'biometry' };
    },
    unlockApp: () => new Promise((resolve) => {
      calls.push('unlock');
      finishUnlock = resolve;
    }),
    agentStatus: async () => {
      calls.push('agent-status');
      return { phase: 'Ready' as const };
    },
  };
  testingLibrary.render(createElement(App, { bridge }));
  await testingLibrary.waitFor(() => assert.match(text('.app-lock'), /Unlock FOKS/));
  assert.equal(document.querySelector('.app-lock-brand'), null);
  assert.match(
    text('.app-lock'),
    /Authenticate with Touch ID or your Mac password to allow FOKS to connect to servers and read vault data\./,
  );
  assert.deepEqual(calls, ['lock-state']);
  const unlockButton = [...document.querySelectorAll<HTMLButtonElement>('.app-lock button')]
    .find((button) => button.textContent === 'Unlock')!;
  testingLibrary.fireEvent.click(unlockButton);
  await testingLibrary.waitFor(() => {
    assert.equal(unlockButton.disabled, true);
    assert.equal(unlockButton.textContent, 'Unlock');
  });
  await testingLibrary.act(async () => {
    locked = false;
    finishUnlock!({ locked, available: true, mechanism: 'biometry' });
  });
  await testingLibrary.waitFor(() => assert.ok(document.querySelector('.window')));
  assert.ok(document.querySelector('.native-window'));
  assert.equal(document.querySelector('.native-window .lights'), null);
  assert.deepEqual(calls.slice(0, 4), [
    'lock-state',
    'unlock',
    'lock-state',
    'agent-status',
  ]);
});

test('native Bootstrap initializes once and enters first run in the same window', async () => {
  window.localStorage.removeItem('foks.first-run.v1');
  window.history.replaceState(null, '', '/?state=all');
  const base = mockBridge(FIXTURE);
  let initialized = 0;
  const bridge = {
    ...nativeAvailableBridge(base),
    agentStatus: async () => ({
      phase: 'Bootstrap' as const,
      step: 'create-state',
    }),
    initializeClientState: async () => {
      initialized += 1;
      return { phase: 'Ready' as const };
    },
  };
  testingLibrary.render(createElement(App, { bridge }));
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /Who is setting you up/),
  );
  assert.equal(initialized, 1);
  assert.equal(document.querySelectorAll('.window').length, 1);
});

test('native Bootstrap reloads the world and enters managed local setup', async () => {
  window.localStorage.removeItem('foks.first-run.v1');
  window.history.replaceState(null, '', '/?state=all');
  const localWorld: World = {
    ...FIXTURE,
    servers: [{ ...FIXTURE.servers[0], id: 'local', name: 'localhost:4430', accounts: [], state: 'ok' }],
    stores: [],
    accounts: [],
    items: [],
  };
  const base = mockBridge(localWorld);
  const native = nativeAvailableBridge(base);
  let initialized = false;
  const bridge: Bridge = {
    ...native,
    agentStatus: async () => initialized
      ? { phase: 'Ready' as const }
      : { phase: 'Bootstrap' as const, step: 'create-state' },
    appInfo: async () => ({
      version: '0.3.0',
      agentSocket: '/private/foks/agent.sock',
      managedProfile: 'local',
    }),
    initializeClientState: async () => {
      initialized = true;
      return { phase: 'Ready' as const };
    },
    describeServerStatus: async (profile) => ({
      ...(await native.describeServerStatus(profile)),
      configuredProbe: 'localhost:4430',
      leaseRequired: false,
      leaseExpiresAt: null,
    }),
  };
  testingLibrary.render(createElement(App, { bridge }));
  await testingLibrary.waitFor(() =>
    assert.match(text('.first-run-main'), /A private FOKS server is already running on this Mac/),
  );
  assert.equal(initialized, true);
  assert.match(text('.titlebar .agent'), /Agent ready/);
});

test('a failed native initialization has an explicit retry without concurrent replay', async () => {
  window.localStorage.removeItem('foks.first-run.v1');
  window.history.replaceState(null, '', '/?state=all');
  const base = mockBridge(FIXTURE);
  let initialized = 0;
  const bridge = {
    ...nativeAvailableBridge(base),
    agentStatus: async () => ({ phase: 'Bootstrap' as const }),
    initializeClientState: async () => {
      initialized += 1;
      if (initialized === 1) throw new Error('The local helper did not start.');
      return { phase: 'Ready' as const };
    },
  };
  testingLibrary.render(createElement(App, { bridge }));
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /local helper did not start/i),
  );
  assert.equal(initialized, 1);
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('button')].find(
      (button) => button.textContent === 'Try again',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /Who is setting you up/),
  );
  assert.equal(initialized, 2);
});

test('a ready native client shows noninteractive first-run progress without initializing again', async () => {
  window.localStorage.removeItem('foks.first-run.v1');
  window.history.replaceState(null, '', '/?state=all');
  const base = mockBridge(FIXTURE);
  let initializationAttempts = 0;
  const bridge = {
    ...nativeAvailableBridge(base),
    agentStatus: async () => ({ phase: 'Ready' as const }),
    initializeClientState: async () => {
      initializationAttempts += 1;
      throw new Error(
        'Refresh the vault to reconcile the previous change before making another one.',
      );
    },
    listCatalog: async () => ({
      profiles: [],
      stores: [],
      items: [],
      failures: [],
      blockedProfiles: [],
    }),
    listServers: async () => [],
    listAccounts: async () => [],
  };
  testingLibrary.render(createElement(App, { bridge }));
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /Who is setting you up/),
  );
  const steps = [...document.querySelectorAll<HTMLElement>('.setup-step')];
  assert.equal(steps.length, 7);
  assert.equal(document.querySelectorAll('button.setup-step').length, 0);
  assert.equal(
    steps.find((step) => step.textContent?.includes('Who is setting you up?'))
      ?.getAttribute('aria-current'),
    'step',
  );
  testingLibrary.fireEvent.click(
    steps.find((step) => step.textContent?.includes('Preparing this Mac'))!,
  );
  assert.match(text('.main'), /Who is setting you up/);
  assert.equal(initializationAttempts, 0);
});

test('managed local setup uses concise copy and noninteractive progress steps', () => {
  window.localStorage.removeItem('foks.first-run.v1');
  at('?state=first-run&step=local&path=own');
  assert.equal(document.querySelectorAll('.setup-step').length, 3);
  assert.equal(document.querySelectorAll('button.setup-step').length, 0);
  assert.equal(
    document.querySelector('.setup-step[aria-current="step"]')?.textContent?.trim(),
    '1Local server',
  );
  assert.equal(text('.local-server-title small'), 'On this Mac');
  assert.match(text('.local-server-facts'), /TrustApp-managed certificate/);
  assert.match(text('.local-server-facts'), /StorageLocal only/);
  assert.match(text('.setup-side .foot'), /Use another server/);
});

test('managed account options stay optional and Recovery returns to the account page', async () => {
  window.localStorage.setItem(
    'foks.first-run.v1',
    encodeFirstRunCheckpoint({
      ...initialFirstRun('own', 'account'),
      managedLocal: true,
      profile: {
        profile: 'local',
        acceptance: 'unchanged',
        lookupName: 'localhost',
        canonicalName: 'localhost',
        hostId: `02${'6'.repeat(64)}`,
        chain: 2,
        epoch: 7,
      },
      serverAddress: 'localhost:4430',
    }),
  );
  at('?state=first-run&step=account&path=own');
  const email = document.querySelector<HTMLInputElement>('input[placeholder="you@example.net"]');
  const username = document.querySelector<HTMLInputElement>('.local-field-card input');
  assert.ok(email);
  assert.ok(username);
  assert.match(text('.local-more'), /Email \(optional\).*Invite \(optional\)/s);
  testingLibrary.fireEvent.change(username, { target: { value: 'changed' } });
  assert.equal(email.placeholder, 'you@example.net');
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('button')].find(
      (button) => button.textContent?.includes('Recover an existing account'),
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.equal(text('.pane h1'), 'Add this Mac to your account'),
  );
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pfoot button')].find(
      (button) => button.textContent?.includes('Back'),
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.equal(text('.pane h1'), 'Create your account'),
  );
});

test('explicit Who does not merge a saved managed-local completion', () => {
  window.localStorage.setItem(
    'foks.first-run.v1',
    encodeFirstRunCheckpoint({
      ...initialFirstRun('own', 'local-done'),
      profile: {
        profile: 'local',
        acceptance: 'unchanged',
        lookupName: 'localhost',
        canonicalName: 'localhost',
        hostId: `02${'6'.repeat(64)}`,
        chain: 2,
        epoch: 7,
      },
      serverAddress: 'localhost:4430',
      account: { alias: 'personal', username: 'sol', deviceName: 'Sol Mac' },
      backupCommitted: true,
    }),
  );
  at('?state=first-run&step=who&path=own');
  assert.match(text('.first-run-main'), /Who is setting you up/);
  assert.equal(document.querySelectorAll('.setup-step').length, 7);
  assert.doesNotMatch(text('.setup-side'), /Local server/);
});

test('managed local completion opens its exact Personal store and reports recovered items', async () => {
  window.localStorage.setItem(
    'foks.first-run.v1',
    encodeFirstRunCheckpoint({
      ...initialFirstRun('own', 'local-done'),
      profile: {
        profile: 'personal',
        acceptance: 'unchanged',
        lookupName: 'foks.example.net',
        canonicalName: 'foks.example.net',
        hostId: `02${'6'.repeat(64)}`,
        chain: 2,
        epoch: 7,
      },
      serverAddress: 'foks.example.net',
      account: { alias: 'personal', username: 'rae', deviceName: 'This Mac' },
      backupCommitted: true,
    }),
  );
  at('?state=first-run&step=local-done&path=own');
  const count = FIXTURE.items.filter((item) => item.store === 'acct:personal').length;
  assert.equal(
    text('.local-vault-empty'),
    `${count} ${count === 1 ? 'item' : 'items'}`,
  );
  const open = [...document.querySelectorAll<HTMLButtonElement>('.pfoot button')]
    .find((button) => button.textContent === 'Open Personal');
  assert.ok(open);
  assert.equal(open.disabled, false);
  testingLibrary.fireEvent.click(open);
  await testingLibrary.waitFor(() =>
    assert.equal(text('.main .loc h1'), 'Personal'),
  );
});

test('managed local setup rejects a profile aimed at another endpoint', async () => {
  window.localStorage.removeItem('foks.first-run.v1');
  window.history.replaceState(null, '', '/?state=all');
  const localWorld: World = {
    ...FIXTURE,
    servers: [{ ...FIXTURE.servers[0], id: 'local', name: 'localhost:4430', accounts: [], state: 'ok' }],
    stores: [],
    accounts: [],
    items: [],
  };
  const base = mockBridge(localWorld);
  const native = nativeAvailableBridge(base);
  const bridge: Bridge = {
    ...native,
    appInfo: async () => ({
      version: '0.3.0',
      agentSocket: '/private/foks/agent.sock',
      managedProfile: 'local',
    }),
    describeServerStatus: async (profile) => ({
      ...(await native.describeServerStatus(profile)),
      configuredProbe: 'remote.example:4430',
      leaseRequired: false,
    }),
  };
  testingLibrary.render(createElement(App, { bridge }));
  await testingLibrary.waitFor(() =>
    assert.match(text('.first-run-main'), /Local server unavailable/),
  );
  const continueButton = [...document.querySelectorAll<HTMLButtonElement>('.pfoot button')]
    .find((button) => button.textContent === 'Continue');
  assert.equal(continueButton?.disabled, true);
});

test('automatic native first-run entry resumes the saved step instead of injected Who', async () => {
  const profile = {
    profile: 'personal',
    acceptance: 'inserted' as const,
    lookupName: 'foks.example.net',
    canonicalName: 'foks.example.net',
    hostId: `02${'6'.repeat(64)}`,
    chain: 2,
    epoch: 7,
  };
  window.localStorage.setItem(
    'foks.first-run.v1',
    encodeFirstRunCheckpoint({
      ...initialFirstRun('own', 'checked'),
      profile,
      serverAddress: 'foks.example.net',
    }),
  );
  window.history.replaceState(null, '', '/?state=all');
  const base = mockBridge(FIXTURE);
  const bridge = {
    ...nativeAvailableBridge(base),
    agentStatus: async () => ({ phase: 'Ready' as const }),
    listCatalog: async () => ({
      profiles: [],
      stores: [],
      items: [],
      failures: [],
      blockedProfiles: [],
    }),
    listServers: async () => [],
    listAccounts: async () => [],
  };
  testingLibrary.render(createElement(App, { bridge }));
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /Found it/),
  );
  const search = new URLSearchParams(window.location.search);
  assert.equal(search.get('state'), 'first-run');
  assert.equal(search.get('step'), 'checked');
  assert.equal(search.get('path'), 'own');
});

test('a large catalog mounts only the measured 50px row window', async () => {
  const seed = FIXTURE.items.find(
    (item) => item.store === 'acct:personal' && item.kind === 'Secret',
  );
  assert.ok(seed);
  const bulk = Array.from({ length: 500 }, (_, index) => ({
    ...seed,
    path: `/bulk/item-${String(index).padStart(4, '0')}`,
    version: index + 1,
  }));
  const world: World = { ...FIXTURE, items: bulk };
  at('?state=all', { world, bridge: mockBridge(world) });
  const body = document.querySelector<HTMLElement>('.body');
  assert.ok(body);
  assert.ok(document.querySelectorAll('.body .row').length < bulk.length);
  testingLibrary.fireEvent.scroll(body, { target: { scrollTop: 10_000 } });
  await testingLibrary.waitFor(() => {
    assert.ok(row('item-0200'));
  });
  assert.equal(
    document
      .querySelector('.body .row .name .tt')
      ?.textContent?.includes('item-0000'),
    false,
  );
});

test('?state=all&lease=lapsed says which stores the lapse took away', () => {
  at('?state=all&lease=lapsed');
  const lapsed = applyLease(FIXTURE, 'lapsed');
  const band = document.querySelector('.body > .band');
  assert.ok(band, 'the lapsed world puts a band over All items');
  // The names are the stores on the lapsed server, not a sentence with names
  // written into it.
  for (const store of lapsed.stores.filter(
    (candidate) => candidate.server === 'acme',
  )) {
    assert.match(
      band.textContent ?? '',
      new RegExp(store.name.replace(/[()]/g, '\\$&')),
    );
  }
  // And nothing from that server is listed.
  assert.equal(
    document.querySelectorAll('.body .row').length,
    catalog(lapsed).length,
  );
});

test('two lapsed profiles get separate bands with only their own stores', () => {
  const world: World = {
    ...FIXTURE,
    leaseState: 'lapsed',
    servers: FIXTURE.servers.map((server) => ({
      ...server,
      state: 'lease-lapsed',
      lease: { state: 'lapsed', expires_in: null },
    })),
  };
  at('?state=all', { world, bridge: mockBridge(world) });
  const bands = [...document.querySelectorAll('.body > .band')].map(
    (band) => band.textContent ?? '',
  );
  assert.equal(
    bands.length,
    new Set(world.stores.map((store) => store.server)).size,
  );
  const personal = bands.find((band) => band.includes('foks.example.net'));
  const acme = bands.find((band) => band.includes('foks.acme-corp.com'));
  assert.ok(personal);
  assert.ok(acme);
  assert.match(personal, /Personal/);
  assert.doesNotMatch(personal, /Work \(Acme\)|Engineering/);
  assert.match(acme, /Work \(Acme\).*Engineering/);
  assert.doesNotMatch(acme, /Personal|Household|Homelab/);
});

/* -------------------------------------------------------------- personal -- */

test('?state=personal lists one account store with its computed reader count', () => {
  at('?state=personal');
  assert.equal(text('.loc h1'), 'Personal');
  assert.equal(text('.loc small'), 'foks.example.net');
  const rows = document.querySelectorAll('.body .row');
  assert.equal(
    rows.length,
    catalog(FIXTURE).filter((item) => item.store === 'acct:personal').length,
  );
  for (const chip of all('.body .row .chip')) assert.equal(chip, '1');
});

test('?state=password opens the masked GitHub login', () => {
  at('?state=password');
  assert.equal(text('.details .dh h2'), 'github.com');
  assert.equal(text('.details .v.mask'), '••••••••••');
  assert.ok(
    [...document.querySelectorAll('button')].some(
      (button) => button.textContent?.trim() === 'Show',
    ),
  );
  assert.deepEqual(
    all('.details .irow .a button'),
    ['Show', 'Copy'],
  );
});

test('password Copy keeps its position and uses the concise notification', async () => {
  at('?state=password');
  const copy = [...document.querySelectorAll<HTMLButtonElement>('.details .irow .a button')]
    .find((button) => button.textContent?.trim() === 'Copy');
  assert.ok(copy);
  testingLibrary.fireEvent.click(copy);
  await testingLibrary.waitFor(() => {
    assert.equal(text('.details .action-message'), 'Password copied');
  });
});

test('item removal uses a concise ordinary confirmation', () => {
  at('?state=password');
  const remove = [...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button')]
    .find((button) => button.textContent?.trim() === 'Remove');
  assert.ok(remove);
  testingLibrary.fireEvent.click(remove);
  assert.equal(text('.sheet .sb'), 'This will remove the item. This can’t be undone.');
  assert.equal(text('.sheet .ft .danger'), 'Remove');
});

test('a native password invents no fields before Show and parses structured content after it', async () => {
  const world: World = {
    ...FIXTURE,
    servers: [
      {
        id: 'native-server',
        name: 'native.example',
        label: null,
        host_id: null,
        chain: null,
        epoch: null,
        lease: null,
        accounts: ['jun'],
        state: 'ok',
      },
    ],
    stores: [
      {
        id: 'native-store',
        kind: 'account',
        name: 'Personal',
        server: 'native-server',
        account: 'jun',
      },
    ],
    items: [
      {
        store: 'native-store',
        path: '/logins/service.example',
        kind: 'Secret',
        size: 61,
        version: 8,
        read: { role: 'Owner' },
        write: { role: 'Owner' },
      },
    ],
    parties: [],
    federation: [],
    notifications: [],
    plaintext: {},
  };
  const bridge = mockBridge(world);
  bridge.readItem = async (request) => ({
    store: request.storeId,
    path: request.path,
    version: request.version,
    value: 'user: jun\npassword: native-secret\nurl: https://service.example',
  });
  at(
    '?state=store&store=native-store&sel=native-store%7C%2Flogins%2Fservice.example',
    {
      world,
      bridge,
    },
  );
  assert.match(text('.details .prev'), /Password••••••••••/);
  assert.doesNotMatch(text('.details .prev'), /jun|service\.example/);
  const show = [
    ...document.querySelectorAll<HTMLButtonElement>('.details button'),
  ].find((button) => button.textContent?.trim() === 'Show');
  assert.ok(show);
  testingLibrary.fireEvent.click(show);
  await testingLibrary.waitFor(() => {
    const preview = text('.details .prev');
    assert.match(preview, /User namejun/);
    assert.match(preview, /Password.*native-secret/);
    assert.match(preview, /Websitehttps:\/\/service\.example/);
  });
});

test('?state=resource opens the masked API key resource', () => {
  at('?state=resource');
  assert.equal(text('.details .dh h2'), 'anthropic-api-key');
  assert.doesNotMatch(text('.details'), /ValueValue/);
  assert.equal(text('.details .v.mask'), '••••••••••••••••••••');
});

test('?state=file offers a version-bound native download', () => {
  at('?state=file');
  assert.equal(text('.details .dh h2'), 'emergency.pdf');
  const download = [
    ...document.querySelectorAll<HTMLButtonElement>('.details button'),
  ].find((button) => button.textContent?.trim() === 'Download');
  assert.ok(download);
  assert.equal(download.disabled, false);
  assert.equal(document.querySelector('.details .pfn'), null);
});

test('a binary small-file can be downloaded and replaced as a native file', async () => {
  const source = FIXTURE.items.find(
    (item) => item.store === 'acct:personal' && item.path === '/env/prod/DATABASE_URL',
  );
  assert.ok(source);
  const item: Item = { ...source, path: '/certs/client.p12', value: undefined };
  const world: World = { ...FIXTURE, items: [item] };
  const bridge = mockBridge(world);
  bridge.readItem = async () => {
    throw {
      code: 'not-text',
      message: 'This file is not UTF-8 text. Download it instead.',
      retryable: false,
      ambiguous: false,
      fatal: false,
    };
  };
  let replacement: Parameters<Bridge['pickAndReplaceFile']>[0] | null = null;
  bridge.pickAndReplaceFile = async (request) => {
    replacement = request;
    return { applied: true };
  };

  at(`?state=store&store=acct%3Apersonal&sel=${encodeURIComponent(`${item.store}|${item.path}`)}`, {
    world,
    bridge,
  });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.details button')]
      .find((button) => button.textContent?.trim() === 'Show')!,
  );
  await testingLibrary.waitFor(() => {
    assert.match(text('.details .dh'), /File in Personal/);
    assert.ok([...document.querySelectorAll<HTMLButtonElement>('.details button')]
      .some((button) => button.textContent?.trim() === 'Download'));
  });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button')]
      .find((button) => button.textContent?.trim() === 'Edit')!,
  );
  await testingLibrary.waitFor(() => assert.match(text('.details .prev'), /Replace/));
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button')]
      .find((button) => button.textContent?.startsWith('Save version'))!,
  );
  await testingLibrary.waitFor(() => assert.ok(replacement));
  assert.deepEqual(replacement, {
    storeId: item.store,
    path: item.path,
    version: item.version,
  });
});

test('?state=link keeps the target masked until an exact-version read', async () => {
  const bridge = mockBridge(FIXTURE);
  let calls = 0;
  let request: unknown;
  const readItem = bridge.readItem.bind(bridge);
  bridge.readItem = async (next) => {
    calls += 1;
    request = next;
    return readItem(next);
  };
  at('?state=link', { bridge });
  assert.equal(calls, 0);
  assert.match(text('.details .fileglyph'), /target masked until read/);
  const readTarget = [
    ...document.querySelectorAll<HTMLButtonElement>('.details button'),
  ].find((button) => button.textContent?.trim() === 'Read target');
  assert.ok(readTarget);
  testingLibrary.fireEvent.click(readTarget);
  await testingLibrary.waitFor(() => {
    assert.match(text('.details .fileglyph'), /points to \/ssh\/id_ed25519/);
  });
  assert.equal(calls, 1);
  assert.deepEqual(request, {
    storeId: 'acct:personal',
    path: '/latest-key',
    version: 2,
  });
});

test('blur drops a learned link target and does not read it again implicitly', async () => {
  const bridge = mockBridge(FIXTURE);
  let calls = 0;
  const readItem = bridge.readItem.bind(bridge);
  bridge.readItem = async (request) => {
    calls += 1;
    return readItem(request);
  };
  at('?state=link', { bridge });
  const readTarget = [
    ...document.querySelectorAll<HTMLButtonElement>('.details button'),
  ].find((button) => button.textContent?.trim() === 'Read target');
  assert.ok(readTarget);
  testingLibrary.fireEvent.click(readTarget);
  await testingLibrary.waitFor(() =>
    assert.ok(document.querySelector('.details .fileglyph code')),
  );
  testingLibrary.fireEvent(window, new Event('blur'));
  await testingLibrary.waitFor(() => {
    assert.equal(document.querySelector('.details .fileglyph code'), null);
    assert.match(text('.details .fileglyph'), /target masked until read/);
  });
  assert.equal(calls, 1);
});

/* ----------------------------------------------------------------- group -- */

test('?state=group matches the Household Wi-Fi sharing view', () => {
  at('?state=group');
  assert.equal(text('.loc h1'), 'Household');
  assert.equal(text('.details .dh h2'), 'guest-password');

  const item = FIXTURE.items.find(
    (candidate) => itemKey(candidate) === 'team:household|/wifi/guest-password',
  );
  assert.ok(item);
  const readers = readersOf(FIXTURE, item);
  assert.ok(readers);
  assert.match(
    text('.details .who p'),
    new RegExp(`^${peopleLabel(readers.length)} can read this`),
  );
});

test('Engineering computes Readable by from the roster and the read role', () => {
  at('?state=store&store=team:eng');
  assert.equal(text('.loc h1'), 'Engineering');

  const roster = peopleGroups(partiesOf(FIXTURE, 'team:eng'));
  assert.equal(
    roster,
    '5 people · 1 group',
    'the fixture roster, as the plan writes it',
  );
  assert.ok(
    text('.loc small').startsWith(roster),
    `the header carries the roster summary: ${text('.loc small')}`,
  );

  // Admin-read: the two Admins and the Owner. The admitted group's admission
  // reports inactive, so it reads nothing here however good its role looks.
  const production = readerChip(FIXTURE, 'team:eng|/deploy/production-token');
  assert.equal(production, '3 people');
  assert.equal(
    row('production-token').querySelector('.chip')?.textContent,
    '3',
  );

  // Member · visibility 0: everyone but the inactive admission.
  const staging = readerChip(FIXTURE, 'team:eng|/deploy/staging-token');
  assert.equal(staging, '5 people');
  assert.equal(
    row('staging-token').querySelector('.chip')?.textContent,
    '5',
  );

  assert.equal(
    document.querySelector('.body > .band'),
    null,
    'the expired group-write rollout band is gone',
  );
});

/* ------------------------------------------------------------------ grid -- */

test('?state=grid draws cards in store sections, not rows', () => {
  at('?state=grid');
  assert.equal(text('.loc h1'), 'All items');
  assert.equal(document.querySelector('.hdr'), null, 'no list header');
  assert.equal(
    document.querySelectorAll('.tile').length,
    catalog(FIXTURE).length,
  );
  // A section per store that lists something, and a group section says who
  // is in it.
  const sections = all('.gsec');
  // One section per store that lists something.
  assert.equal(
    sections.length,
    new Set(catalog(FIXTURE).map((item) => item.store)).size,
  );
  assert.ok(
    sections.some((section) =>
      section.includes(peopleGroups(partiesOf(FIXTURE, 'team:eng'))),
    ),
    `the Engineering section carries its roster: ${sections.join(' | ')}`,
  );
});

/* ----------------------------------------------------------------- lease -- */

/* --------------------------------------------------------- write states -- */

test('?state=new draws the Password sheet with live group creation', () => {
  at('?state=new');
  assert.ok(
    document.querySelector('[role="dialog"][aria-label="New password"]'),
  );
  assert.equal(text('.sheet h2'), 'New password');
  assert.match(text('.sheet'), /Household/);
  assert.equal(document.querySelector('.sheet .band'), null);
  assert.equal(
    document.querySelector<HTMLInputElement>(
      '.sheet input[aria-label="Password"]',
    )?.type,
    'password',
  );
  const create = [
    ...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button'),
  ].at(-1);
  assert.equal(create?.disabled, false);
});

test('?state=new-group computes the Engineering read preview from its roster', () => {
  at('?state=new-group');
  const draft: Item = {
    store: 'team:eng',
    path: '/preview',
    kind: 'Secret',
    size: 0,
    version: 0,
    read: { role: 'Member', visibility: 0 },
    write: { role: 'Admin' },
  };
  const readers = readersOf(FIXTURE, draft);
  assert.ok(readers);
  assert.equal(text('.sheet h2'), 'New resource');
  assert.match(
    text('.sheet'),
    new RegExp(
      `would be readable by ${readers.length} of ${partiesOf(FIXTURE, 'team:eng').length}`,
    ),
  );
  assert.equal(
    document.querySelector<HTMLButtonElement>('.sheet .ft .primary')?.disabled,
    false,
  );
  assert.ok(document.querySelector('.sheet input[aria-label="Read visibility"]'));
});

test('an active group text create carries both selected roles and refreshes the computed reader count', async () => {
  const base = mockBridge(FIXTURE);
  const creates: CreateTextRequest[] = [];
  const bridge: Bridge = {
    ...base,
    createTextItem: async (request) => {
      creates.push(request);
      return base.createTextItem(request);
    },
  };
  at('?state=group-new-text', { bridge });
  testingLibrary.fireEvent.click(
    document.querySelector<HTMLButtonElement>('[aria-label="Read role Admin"]')!,
  );
  testingLibrary.fireEvent.change(
    document.querySelector<HTMLInputElement>('input[aria-label="Name"]')!,
    { target: { value: 'phase7-shared' } },
  );
  testingLibrary.fireEvent.change(
    document.querySelector<HTMLInputElement>('input[aria-label="Value"]')!,
    { target: { value: 'shared value' } },
  );
  const candidate: Item = {
    store: 'team:eng', path: '/preview', kind: 'Secret', size: 0, version: 0,
    read: { role: 'Admin' }, write: { role: 'Admin' },
  };
  const expected = readersOf(FIXTURE, candidate)?.length;
  assert.notEqual(expected, undefined);
  assert.match(text('.sheet'), new RegExp(`would be readable by ${expected} of ${partiesOf(FIXTURE, 'team:eng').length}`));
  testingLibrary.fireEvent.click(
    document.querySelector<HTMLButtonElement>('.sheet .ft .primary')!,
  );
  await testingLibrary.waitFor(() => assert.equal(creates.length, 1));
  assert.deepEqual(creates[0], {
    storeId: 'team:eng',
    path: '/agents/phase7-shared',
    value: 'shared value',
    readRole: 'Admin',
    writeRole: 'Admin',
  });
  await testingLibrary.waitFor(() =>
    assert.match(text('.flash'), /Resource created in Engineering/),
  );
  const rendered = row('phase7-shared');
  assert.equal(rendered.querySelector('.chip')?.textContent, String(expected));
});

test('group Member role controls carry the full canonical signed i16 range', async () => {
  const base = mockBridge(FIXTURE);
  const creates: CreateTextRequest[] = [];
  const bridge: Bridge = {
    ...base,
    createTextItem: async (request) => {
      creates.push(request);
      return base.createTextItem(request);
    },
  };
  at('?state=group-new-text', { bridge });
  const read = document.querySelector<HTMLInputElement>('input[aria-label="Read visibility"]');
  assert.ok(read);
  assert.equal(read.min, '-32768');
  assert.equal(read.max, '32767');
  testingLibrary.fireEvent.change(read, { target: { value: '-32768' } });
  testingLibrary.fireEvent.click(
    document.querySelector<HTMLButtonElement>('[aria-label="Write role Member"]')!,
  );
  const write = document.querySelector<HTMLInputElement>('input[aria-label="Write visibility"]');
  assert.ok(write);
  assert.equal(write.min, '-32768');
  assert.equal(write.max, '32767');
  testingLibrary.fireEvent.change(write, { target: { value: '32767' } });
  testingLibrary.fireEvent.change(
    document.querySelector<HTMLInputElement>('input[aria-label="Name"]')!,
    { target: { value: 'role-boundaries' } },
  );
  testingLibrary.fireEvent.change(
    document.querySelector<HTMLInputElement>('input[aria-label="Value"]')!,
    { target: { value: 'value' } },
  );
  testingLibrary.fireEvent.click(
    document.querySelector<HTMLButtonElement>('.sheet .ft .primary')!,
  );
  await testingLibrary.waitFor(() => assert.equal(creates.length, 1));
  assert.equal(creates[0]?.readRole, 'Member:-32768');
  assert.equal(creates[0]?.writeRole, 'Member:32767');
});

test('group Link creation authors the product symlink with both selected roles', async () => {
  const base = mockBridge(FIXTURE);
  let created: Parameters<Bridge['createLink']>[0] | null = null;
  const bridge: Bridge = {
    ...base,
    createLink: async (request) => {
      created = request;
      return base.createLink(request);
    },
  };
  at('?state=group-new-link', { bridge });
  testingLibrary.fireEvent.click(
    document.querySelector<HTMLButtonElement>('[aria-label="Read role Owner"]')!,
  );
  const ownerReaders = readersOf(FIXTURE, {
    store: 'team:eng', path: '/preview', kind: 'Link', size: 0, version: 0,
    read: { role: 'Owner' }, write: { role: 'Admin' },
  });
  const adminChangers = readersOf(FIXTURE, {
    store: 'team:eng', path: '/preview', kind: 'Link', size: 0, version: 0,
    read: { role: 'Admin' }, write: { role: 'Admin' },
  });
  assert.ok(ownerReaders);
  assert.ok(adminChangers);
  assert.match(
    text('.sheet'),
    new RegExp(`would be readable by ${ownerReaders.length} of .*changeable by ${adminChangers.length} of`, 's'),
  );
  assert.match(text('.sheet'), /roles are independent.*might not be allowed to read/s);
  testingLibrary.fireEvent.change(
    document.querySelector<HTMLInputElement>('input[aria-label="Points to"]')!,
    { target: { value: '/deploy/staging-token' } },
  );
  testingLibrary.fireEvent.click(
    document.querySelector<HTMLButtonElement>('.sheet .ft .primary')!,
  );
  await testingLibrary.waitFor(() => assert.ok(created));
  assert.deepEqual(created, {
    storeId: 'team:eng',
    path: '/latest-key',
    target: '/deploy/staging-token',
    readRole: 'Owner',
    writeRole: 'Admin',
  });
});

test('an active group without one authenticated local identity cannot create', () => {
  const world: World = {
    ...FIXTURE,
    parties: FIXTURE.parties.map((party) =>
      party.store === 'team:eng' && party.label === 'you'
        ? { ...party, label: undefined }
        : party,
    ),
  };
  at('?state=group-new-text', { world });
  assert.match(text('.sheet'), /no authenticated local group identity/);
  assert.equal(
    document.querySelector<HTMLButtonElement>('.sheet .ft .primary')?.disabled,
    true,
  );
});

test('group native-picker file creation carries roles while renderer receives no bytes', async () => {
  const base = mockBridge(FIXTURE);
  let created: Parameters<Bridge['pickAndImportFile']>[0] | null = null;
  const bridge: Bridge = {
    ...base,
    pickAndImportFile: async (request) => {
      created = request;
      return base.pickAndImportFile(request);
    },
  };
  at('?state=group-new-file', { bridge });
  assert.equal(document.querySelector('.sheet input[type="file"]'), null);
  testingLibrary.fireEvent.click(
    document.querySelector<HTMLButtonElement>('.sheet .ft .primary')!,
  );
  await testingLibrary.waitFor(() => assert.ok(created));
  assert.deepEqual(created, {
    storeId: 'team:eng',
    path: '/documents/emergency.pdf',
    readRole: 'Member:0',
    writeRole: 'Admin',
  });
});

test('?state=new-resource enables a concise account create', () => {
  at('?state=new-resource');
  assert.equal(text('.sheet h2'), 'New resource');
  assert.doesNotMatch(text('.sheet'), /must not exist/i);
  assert.equal(
    document.querySelector<HTMLButtonElement>('.sheet .ft .primary')?.disabled,
    false,
  );
});

test('?state=new-file keeps bytes out of the renderer and enables group import', () => {
  at('?state=new-file');
  assert.equal(text('.sheet h2'), 'New file');
  assert.match(text('.sheet'), /Drop a file here, or use the native picker/);
  assert.equal(document.querySelector('.sheet input[type="file"]'), null);
  assert.equal(
    document.querySelector<HTMLButtonElement>('.sheet .ft .primary')?.disabled,
    false,
  );
});

test('?state=new-link draws target and path fields in Personal', () => {
  at('?state=new-link');
  assert.equal(text('.sheet h2'), 'New link');
  assert.ok(document.querySelector('.sheet input[aria-label="Points to"]'));
  assert.ok(document.querySelector('.sheet input[aria-label="Path"]'));
  assert.equal(
    document.querySelector<HTMLButtonElement>('.sheet .ft .primary')?.disabled,
    false,
  );
});

test('?state=exists refuses replacement and opens the colliding exact version', async () => {
  at('?state=exists');
  assert.match(
    text('.sheet h2'),
    /Something is already at \/logins\/github\.com/,
  );
  assert.match(
    text('.sheet'),
    /Nothing was created and nothing was overwritten/,
  );
  assert.doesNotMatch(text('.sheet .ft'), /anyway/i);
  const open = [
    ...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button'),
  ].find((button) => button.textContent?.startsWith('Open'));
  assert.ok(open);
  await testingLibrary.act(async () => {
    testingLibrary.fireEvent.click(open);
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  await testingLibrary.waitFor(() =>
    assert.equal(text('.details .dh h2'), 'github.com'),
  );
  await testingLibrary.act(async () => testingLibrary.cleanup());
});

test('Open existing refreshes the invalidated catalog before selecting it', async () => {
  const base = mockBridge(FIXTURE);
  const bridge = {
    ...base,
    listCatalog: async () => {
      const response = await base.listCatalog();
      return {
        ...response,
        items: response.items.map((item) =>
          item.store === 'acct:personal' && item.path === '/logins/github.com'
            ? { ...item, version: item.version + 1 }
            : item,
        ),
      };
    },
  };
  at('?state=exists', { bridge });
  await testingLibrary.act(async () => {
    testingLibrary.fireEvent.click(
      [...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find(
        (button) => button.textContent?.startsWith('Open version'),
      )!,
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  await testingLibrary.waitFor(() =>
    assert.equal(text('.details .dh h2'), 'github.com'),
  );
  const original = FIXTURE.items.find(
    (item) =>
      item.store === 'acct:personal' && item.path === '/logins/github.com',
  );
  assert.ok(original);
  assert.match(
    text('.details .meta'),
    new RegExp(`Version${original.version + 1}`),
  );
});

test('?state=conflict names the inspected version, invents no current version, and does not re-read', async () => {
  const bridge = mockBridge(FIXTURE);
  let reads = 0;
  bridge.readItem = async (request) => {
    reads += 1;
    return mockBridge(FIXTURE).readItem(request);
  };
  at('?state=conflict', { bridge });
  const item = FIXTURE.items.find(
    (candidate) => itemKey(candidate) === 'acct:personal|/logins/github.com',
  );
  assert.ok(item);
  assert.match(text('.sheet'), new RegExp(`edited version ${item.version}`));
  assert.doesNotMatch(
    text('.sheet'),
    new RegExp(`now (holds )?version ${item.version + 1}`),
  );
  assert.match(text('.sheet .ft'), /Discard my edit.*Refresh and review/);
  await testingLibrary.act(async () => Promise.resolve());
  assert.equal(reads, 0);
});

test('conflict Discard exits edit and Refresh restores the retained draft for review', async () => {
  at('?state=conflict');
  const draft = FIXTURE.items
    .find((item) => itemKey(item) === 'acct:personal|/logins/github.com')
    ?.value?.replace(
      /password: .*/,
      `password: ${FIXTURE.plaintext['acct:personal|/logins/github.com']}`,
    );
  assert.ok(draft);
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find(
      (button) => button.textContent === 'Refresh and review',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.equal(
      document.querySelector<HTMLTextAreaElement>('.details textarea')?.value,
      draft,
    ),
  );

  testingLibrary.cleanup();
  at('?state=conflict');
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find(
      (button) => button.textContent === 'Discard my edit',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.equal(document.querySelector('.details textarea'), null),
  );
});

test('?state=agent-lost is a full-window stop with reconnect Retry', () => {
  at('?state=agent-lost');
  assert.ok(
    document.querySelector(
      '[role="alertdialog"][aria-label="Local agent connection lost"]',
    ),
  );
  assert.match(text('.stopwrap'), /We encountered a connection error/);
  assert.equal(text('.stopwrap .primary'), 'Retry');
  assert.equal(document.querySelector('.backdrop'), null);
});

test('?state=lease replaces the list with the lapsed check-in notice', () => {
  at('?state=lease');
  assert.equal(text('.loc h1'), 'Work (Acme)');
  assert.equal(document.querySelector('.body .row'), null, 'nothing is listed');
  const notice = document.querySelector('.notice');
  assert.ok(notice);
  assert.equal(notice.querySelector(':scope > .who'), null);
  const rows = [...document.querySelectorAll('.side .nav')];
  const work = rows.find((candidate) =>
    candidate.querySelector('.t')?.textContent?.startsWith('Work (Acme)'),
  );
  assert.ok(work);
  assert.match(work.className, /off/);
});

/* -------------------------------------------------------------- inactive -- */

test('?state=inactive says the group is inactive', () => {
  at('?state=inactive');
  assert.equal(text('.loc h1'), 'Homelab');
  assert.equal(document.querySelector('.body .row'), null);
  const notice = document.querySelector('.notice');
  assert.ok(notice);
  assert.equal(notice.querySelector('.fn'), null);
  const rows = [...document.querySelectorAll('.side .nav')];
  const homelab = rows.find((candidate) =>
    candidate.querySelector('.t')?.textContent?.startsWith('Homelab'),
  );
  assert.ok(homelab);
  assert.doesNotMatch(text('.window'), /\bteams?\b/i);
});

test('the inactive-group action invokes the group resumable, not a made-up create', async () => {
  const base = mockBridge(FIXTURE);
  let resumed: string | null = null;
  const bridge = {
    ...base,
    resumeGroupCreation: async (storeId: string) => {
      resumed = storeId;
      return base.resumeGroupCreation(storeId);
    },
  };
  at('?state=inactive', { bridge });
  const button = document.querySelector<HTMLButtonElement>('.notice button');
  assert.ok(button);
  assert.equal(button.disabled, false);
  testingLibrary.fireEvent.click(button);
  await testingLibrary.waitFor(() => assert.equal(resumed, 'team:homelab'));
  await testingLibrary.waitFor(() =>
    assert.equal(document.querySelector('.notice'), null),
  );
});

test('a native multi-file drop is refused before any import command', async () => {
  const base = mockBridge(FIXTURE);
  let imports = 0;
  const bridge = {
    ...base,
    onDropPaths: async (listener: (paths: string[]) => void) => {
      queueMicrotask(() => listener(['/tmp/a', '/tmp/b']));
      return () => {};
    },
    importDroppedFile: async (
      ...args: Parameters<typeof base.importDroppedFile>
    ) => {
      imports += 1;
      return base.importDroppedFile(...args);
    },
  };
  at('?state=new-file', { bridge });
  await testingLibrary.waitFor(() =>
    assert.match(text('.backdrop [role="alert"]'), /Drop one file at a time/),
  );
  assert.equal(imports, 0);
});

/* ---------------------------------------------------------------- issues -- */

test('?state=issues lists the notes that apply in the lapsed world', () => {
  at('?state=issues');
  const lapsed = applyLease(FIXTURE, 'lapsed');
  const notes = notesNow(lapsed);
  assert.equal(document.querySelectorAll('.cards .card').length, notes.length);
  assert.equal(document.querySelector('.loc small'), null);
  assert.equal(text('.side .badge'), String(notes.length));
  assert.equal(document.querySelector('.main > .toolbar'), null);
  assert.doesNotMatch(text('.main'), /Nothing here refreshes on its own/);
  assert.match(
    text('.main'),
    /lease for foks\.acme-corp\.com has expired.*Group setup incomplete\. Items and members are unavailable until setup is finished\..*Homelab’s membership in Engineering is inactive/s,
  );
  assert.doesNotMatch(
    text('.main'),
    /summary carries one state|reports itself inactive|finds out why|admitted Homelab as a member/,
  );
  // A note's severity is drawn, not decided here: the lapsed lease is the
  // critical one in this world.
  assert.equal(
    document.querySelectorAll('.card .sev.crit').length,
    notes.filter((note) => note.severity === 'crit').length,
  );
});

test('the Issues badge is absent at zero, not a "0"', () => {
  // A world with nothing outstanding — the fixture with its notes taken away.
  at('?state=all', { world: { ...FIXTURE, notifications: [] } });
  assert.equal(document.querySelector('.side .badge'), null);
  assert.match(text('.side'), /Issues/, 'the row itself is still there');
});

/* ------------------------------------------------------------------ show -- */

test('?state=show reads exactly the selected version and reveals it', async () => {
  const bridge = mockBridge(FIXTURE);
  const called: unknown[] = [];
  const readItem = bridge.readItem.bind(bridge);
  bridge.readItem = async (request) => {
    called.push(request);
    return readItem(request);
  };
  at('?state=show', { bridge });
  const panel = document.querySelector('.details');
  assert.ok(panel, 'the details panel is open');
  assert.equal(panel.querySelector('.dh h2')?.textContent, 'github.com');
  assert.match(document.querySelector('.app')?.className ?? '', /with-details/);

  const item = FIXTURE.items.find(
    (candidate) => itemKey(candidate) === 'acct:personal|/logins/github.com',
  );
  assert.ok(item);
  const meta = panel.querySelector('.meta')?.textContent ?? '';
  assert.match(meta, new RegExp(item.path));
  assert.match(meta, new RegExp(`Version${item.version}`));
  assert.match(meta, /Read roleOwner/);
  await testingLibrary.waitFor(() => {
    assert.equal(
      panel.querySelector('.v.mono')?.textContent,
      FIXTURE.plaintext[itemKey(item)],
    );
  });
  assert.deepEqual(called, [
    {
      storeId: item.store,
      path: item.path,
      version: item.version,
    },
  ]);
  assert.equal(document.querySelector('.details .pfn'), null);
  assert.deepEqual(all('.details .irow .a button'), ['Hide', 'Copy']);

  // The selected row is the one the panel is describing.
  assert.equal(
    row('github.com').className.includes('sel'),
    true,
    'the row is highlighted',
  );
});

test('blur drops a revealed value and Show can read it again', async () => {
  at('?state=show');
  await testingLibrary.waitFor(() =>
    assert.ok(document.querySelector('.details .v.mono')),
  );
  testingLibrary.fireEvent(window, new Event('blur'));
  await testingLibrary.waitFor(() => {
    assert.equal(text('.details .v.mask'), '••••••••••');
  });
});

test('proactive connection-loss observation stops the window and drops a shown value', async () => {
  const base = mockBridge(FIXTURE);
  let reportLoss: ((message: string | null) => void) | undefined;
  const bridge = {
    ...base,
    takeAgentConnectionLoss: () =>
      new Promise<string | null>((resolve) => {
        reportLoss = resolve;
      }),
  };
  at('?state=show', { bridge });
  await testingLibrary.waitFor(() =>
    assert.ok(document.querySelector('.details .v.mono')),
  );
  await testingLibrary.act(async () => reportLoss?.('transport closed'));
  await testingLibrary.waitFor(() => {
    assert.match(text('.stopwrap'), /Your data has been saved/);
    assert.doesNotMatch(text('.stopwrap'), /transport closed/);
  });
  assert.equal(document.querySelector('.details .v.mono'), null);
});

test('Copy path stays in Rust and names the selected exact version', async () => {
  const bridge = mockBridge(FIXTURE);
  const called: unknown[] = [];
  bridge.copyItemPath = async (request) => {
    called.push(request);
    return { ok: true };
  };
  at('?state=password', { bridge });
  const copy = [
    ...document.querySelectorAll<HTMLButtonElement>('.dfoot button'),
  ].find((button) => button.textContent?.trim() === 'Copy path');
  assert.ok(copy);
  testingLibrary.fireEvent.click(copy);
  await testingLibrary.waitFor(() =>
    assert.match(text('.action-message'), /Path copied/),
  );
  assert.deepEqual(called, [
    {
      storeId: 'acct:personal',
      path: '/logins/github.com',
      version: 9,
    },
  ]);
});

test('Edit reads then saves only the inspected ExactVersion', async () => {
  const base = mockBridge(FIXTURE);
  let written: {
    storeId: string;
    path: string;
    version: number;
    value: string;
  } | null = null;
  const bridge = {
    ...base,
    editTextItem: async (request: {
      storeId: string;
      path: string;
      version: number;
      value: string;
    }) => {
      written = request;
      return base.editTextItem(request);
    },
  };
  at('?state=password', { bridge });
  testingLibrary.fireEvent.click(
    [
      ...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button'),
    ].find((button) => button.textContent?.trim() === 'Edit')!,
  );
  const editor = await testingLibrary.waitFor(() => {
    const found =
      document.querySelector<HTMLTextAreaElement>('.details textarea');
    assert.ok(found);
    return found;
  });
  const next = `${editor.value}\nnote: changed`;
  testingLibrary.fireEvent.change(editor, { target: { value: next } });
  testingLibrary.fireEvent.click(
    [
      ...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button'),
    ].find((button) => button.textContent?.startsWith('Save version'))!,
  );
  await testingLibrary.waitFor(() => assert.ok(written));
  const item = FIXTURE.items.find(
    (candidate) => itemKey(candidate) === 'acct:personal|/logins/github.com',
  );
  assert.ok(item);
  assert.deepEqual(written, {
    storeId: item.store,
    path: item.path,
    version: item.version,
    value: next,
  });
});

test('group text Edit is live and preserves roles by sending only the exact version and value', async () => {
  const base = mockBridge(FIXTURE);
  const item = FIXTURE.items.find(
    (candidate) => candidate.store === 'team:eng' && candidate.path === '/deploy/staging-token',
  );
  assert.ok(item);
  let written: Parameters<Bridge['editTextItem']>[0] | null = null;
  const bridge: Bridge = {
    ...base,
    editTextItem: async (request) => {
      written = request;
      return base.editTextItem(request);
    },
  };
  at(`?state=store&store=team%3Aeng&sel=${encodeURIComponent(`${item.store}|${item.path}`)}`, { bridge });
  const edit = [...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button')]
    .find((button) => button.textContent?.trim() === 'Edit');
  assert.ok(edit);
  assert.equal(edit.disabled, false);
  testingLibrary.fireEvent.click(edit);
  const editor = await testingLibrary.waitFor(() => {
    const found = document.querySelector<HTMLTextAreaElement>('.details textarea');
    assert.ok(found);
    return found;
  });
  testingLibrary.fireEvent.change(editor, { target: { value: 'group replacement' } });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button')]
      .find((button) => button.textContent?.startsWith('Save version'))!,
  );
  await testingLibrary.waitFor(() => assert.ok(written));
  assert.deepEqual(written, {
    storeId: item.store,
    path: item.path,
    version: item.version,
    value: 'group replacement',
  });
});

test('a group item above this account role keeps Edit and Remove disabled', () => {
  const item = FIXTURE.items.find(
    (candidate) => candidate.store === 'team:eng' && candidate.path === '/deploy/production-token',
  );
  assert.ok(item);
  at(`?state=store&store=team%3Aeng&sel=${encodeURIComponent(`${item.store}|${item.path}`)}`);
  const edit = [...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button')]
    .find((button) => button.textContent?.trim() === 'Edit');
  const remove = [...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button')]
    .find((button) => button.textContent?.trim() === 'Remove');
  assert.equal(edit?.disabled, true);
  assert.equal(remove?.disabled, true);
  assert.match(edit?.title ?? '', /does not admit the Owner write role/);
});

test('a streamed group file conflict keeps the exact-version file and never overwrites it', async () => {
  const base = mockBridge(FIXTURE);
  const item = FIXTURE.items.find(
    (candidate) => candidate.store === 'team:eng' && candidate.path === '/release/bundle.tar',
  );
  assert.ok(item);
  let replacement: Parameters<Bridge['replaceDroppedFile']>[0] | null = null;
  const bridge: Bridge = {
    ...base,
    onDropPaths: async (listener) => {
      queueMicrotask(() => listener(['/tmp/new-bundle.tar']));
      return () => {};
    },
    replaceDroppedFile: async (request) => {
      replacement = request;
      throw { code: 'conflict', message: 'changed first', retryable: false, ambiguous: false, fatal: false };
    },
  };
  at(`?state=store&store=team%3Aeng&sel=${encodeURIComponent(`${item.store}|${item.path}`)}`, { bridge });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button')]
      .find((button) => button.textContent?.trim() === 'Edit')!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.details'), /new-bundle\.tar/),
  );
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button')]
      .find((button) => button.textContent?.startsWith('Save version'))!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.sheet h2'), /Someone else changed this first/),
  );
  assert.deepEqual(replacement, {
    storeId: item.store,
    path: item.path,
    version: item.version,
    sourcePath: '/tmp/new-bundle.tar',
  });
  const retained = (await base.listCatalog()).items.find(
    (candidate) => candidate.store === item.store && candidate.path === item.path,
  );
  assert.equal(retained?.version, item.version);
  assert.deepEqual(retained?.read, roleDto(item.read));
  assert.deepEqual(retained?.write, roleDto(item.write));
});

test('Remove confirmation carries the selected version and is danger-last', async () => {
  const base = mockBridge(FIXTURE);
  let removed: { storeId: string; path: string; version: number } | null = null;
  const bridge = {
    ...base,
    removeItem: async (request: {
      storeId: string;
      path: string;
      version: number;
    }) => {
      removed = request;
      return base.removeItem(request);
    },
  };
  at('?state=password', { bridge });
  testingLibrary.fireEvent.click(
    [
      ...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button'),
    ].find((button) => button.textContent?.trim() === 'Remove')!,
  );
  const actions = [
    ...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button'),
  ];
  assert.match(
    actions.at(-1)?.className ?? '',
    /primary.*danger|danger.*primary/,
  );
  testingLibrary.fireEvent.click(actions.at(-1)!);
  await testingLibrary.waitFor(() => assert.ok(removed));
  const item = FIXTURE.items.find(
    (candidate) => itemKey(candidate) === 'acct:personal|/logins/github.com',
  );
  assert.ok(item);
  assert.deepEqual(removed, {
    storeId: item.store,
    path: item.path,
    version: item.version,
  });
});

test('group Remove is live, danger-confirmed, and carries its listed exact version', async () => {
  const base = mockBridge(FIXTURE);
  const item = FIXTURE.items.find(
    (candidate) => candidate.store === 'team:eng' && candidate.path === '/onboarding/README.md',
  );
  assert.ok(item);
  let removed: Parameters<Bridge['removeItem']>[0] | null = null;
  const bridge: Bridge = {
    ...base,
    removeItem: async (request) => {
      removed = request;
      return base.removeItem(request);
    },
  };
  at(`?state=store&store=team%3Aeng&sel=${encodeURIComponent(`${item.store}|${item.path}`)}`, { bridge });
  const open = [...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button')]
    .find((button) => button.textContent?.trim() === 'Remove');
  assert.ok(open);
  assert.equal(open.disabled, false);
  testingLibrary.fireEvent.click(open);
  const actions = [...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button')];
  assert.equal(removed, null, 'opening the sheet does not remove anything');
  testingLibrary.fireEvent.click(actions.at(-1)!);
  await testingLibrary.waitFor(() => assert.ok(removed));
  assert.deepEqual(removed, {
    storeId: item.store,
    path: item.path,
    version: item.version,
  });
});

test('a forced conflict refreshes before restoring the draft under the fresh version', async () => {
  const base = mockBridge(FIXTURE);
  let conflicted = false;
  const bridge = {
    ...base,
    editTextItem: async () => {
      conflicted = true;
      throw {
        code: 'conflict',
        message: 'changed',
        retryable: false,
        ambiguous: false,
        fatal: false,
      };
    },
    listCatalog: async () => {
      const response = await base.listCatalog();
      return {
        ...response,
        items: response.items.map((item) =>
          conflicted &&
          item.store === 'acct:personal' &&
          item.path === '/logins/github.com'
            ? { ...item, version: item.version + 1 }
            : item,
        ),
      };
    },
  };
  at('?state=password', { bridge });
  testingLibrary.fireEvent.click(
    [
      ...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button'),
    ].find((button) => button.textContent?.trim() === 'Edit')!,
  );
  const editor = await testingLibrary.waitFor(() => {
    const found =
      document.querySelector<HTMLTextAreaElement>('.details textarea');
    assert.ok(found);
    return found;
  });
  const draft = `${editor.value}\nnote: retained`;
  testingLibrary.fireEvent.change(editor, { target: { value: draft } });
  testingLibrary.fireEvent.click(
    [
      ...document.querySelectorAll<HTMLButtonElement>('.details .dfoot button'),
    ].find((button) => button.textContent?.startsWith('Save version'))!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.sheet h2'), /Someone else changed/),
  );
  assert.doesNotMatch(text('.sheet'), /version 4/);
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find(
      (button) => button.textContent === 'Refresh and review',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.equal(
      document.querySelector<HTMLTextAreaElement>('.details textarea')?.value,
      draft,
    ),
  );
  const item = FIXTURE.items.find(
    (candidate) => itemKey(candidate) === 'acct:personal|/logins/github.com',
  );
  assert.ok(item);
  assert.match(
    text('.details .pfn'),
    new RegExp(`still version ${item.version + 1}`),
  );
});

/* --------------------------------------------------------------- groups -- */

const GROUP_STATES: ReadonlyArray<[string, string]> = [
  ['groups', 'Your usernames'],
  ['join', 'Join or create a group'],
  ['join-invite', 'Invite to foks.acme-corp.com'],
  ['people', 'People & groups'],
  ['party', 'What deploy-bot can read'],
  ['federation', 'Take it back'],
  ['store', '4 items in Engineering’s store'],
  ['danger', 'Rekeys Engineering'],
  ['invite', 'Invite someone to Engineering'],
  ['add', 'Add someone to Engineering'],
  ['demote', 'Change priya.n’s role'],
  ['remove', 'Remove dana.okafor from Engineering?'],
  ['admit', 'Add a group to Engineering'],
  ['create', 'Create a group'],
  ['manage', 'Manage Household'],
  ['party-remove', 'Remove deploy-bot from Engineering?'],
];

for (const [stateName, phrase] of GROUP_STATES) {
  test(`?state=${stateName} renders its Phase 4 group surface`, () => {
    at(`?state=${stateName}`);
    assert.match(
      text('.window'),
      new RegExp(phrase.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')),
    );
  });
}

test('group cards and usernames use compact server-account identities', () => {
  at('?state=groups');
  const engineeringStore = FIXTURE.stores.find(
    (store) => store.name === 'Engineering',
  );
  assert.ok(engineeringStore);
  const engineeringServer = FIXTURE.servers.find(
    (server) => server.id === engineeringStore.server,
  );
  const engineeringAccount = FIXTURE.accounts.find(
    (account) =>
      account.alias === engineeringStore.account &&
      account.server === engineeringStore.server,
  );
  assert.ok(engineeringServer);
  assert.ok(engineeringAccount);
  const engineering = [
    ...document.querySelectorAll<HTMLElement>('.gcard'),
  ].find((card) => card.textContent?.includes('Engineering'));
  assert.ok(engineering);
  assert.ok(
    engineering.textContent?.includes(
      `${engineeringServer.name} · ${engineeringAccount.username}`,
    ),
  );
  const headerChips = engineering.querySelectorAll('.ghead .chip');
  assert.equal(headerChips.length, 2);
  assert.notEqual(headerChips[0]?.textContent, 'named');
  assert.equal(headerChips[1]?.textContent, 'named');
  assert.doesNotMatch(engineering.textContent ?? '', /via your account|your role/);
  const groups = FIXTURE.stores.filter((store) => store.kind === 'team');
  assert.ok(text('.groups-path').includes(`Groups${groups.length} groupsCreate group`));
  assert.doesNotMatch(text('.groups-path'), /servers/);
  assert.equal(document.querySelector('.groups-usernames .v b'), null);
  assert.doesNotMatch(
    text('.groups-wrap'),
    /Not seeing a group you expect|Ask to be added|Copy the sentence/,
  );
  assert.match(
    text('.groups-wrap'),
    /Group discovery is not available.*currently lists only groups it created or added as members/s,
  );
  assert.doesNotMatch(
    text('.groups-wrap'),
    /Creating it stopped part way|journaled — resuming verifies/,
  );
  assert.match(text('.groups-wrap'), /Creation interrupted.*Resume creation/s);
});

test('the short Join pane offers one account-specific invite per server', () => {
  at('?state=join');
  const invites = [
    ...document.querySelectorAll<HTMLButtonElement>('.plain button'),
  ].filter((button) => button.textContent === 'Invite someone…');
  assert.equal(invites.length, FIXTURE.accounts.length);
  const firstInvite = invites[0];
  const firstServer = FIXTURE.servers[0];
  assert.ok(firstInvite);
  assert.ok(firstServer);
  testingLibrary.fireEvent.click(firstInvite);
  assert.match(
    text('.sheet h2'),
    new RegExp(
      `Invite to ${firstServer.name.replaceAll('.', '\\.')}`,
    ),
  );
});

test('Create defaults to the work account and sends an ad-hoc group without a server name', async () => {
  const base = mockBridge(FIXTURE);
  let created: Parameters<typeof base.createGroup>[0] | null = null;
  at('?state=create', {
    bridge: {
      ...base,
      createGroup: async (request) => {
        created = request;
        return { applied: true };
      },
    },
  });
  assert.match(text('.sheet .hd'), /foks\.acme-corp\.com/);
  assert.doesNotMatch(text('.sheet .hd'), /Household/);
  const adhoc = [
    ...document.querySelectorAll<HTMLButtonElement>('.sheet .radio'),
  ].find((button) => button.textContent?.includes('Ad-hoc'));
  assert.ok(adhoc);
  testingLibrary.fireEvent.click(adhoc);
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button')].find(
      (button) => button.textContent === 'Create ad-hoc group',
    )!,
  );
  await testingLibrary.waitFor(() => assert.ok(created));
  const work = FIXTURE.stores.find(
    (store) => store.kind === 'account' && store.account === 'work',
  );
  assert.ok(work);
  assert.deepEqual(created, {
    accountStoreId: work.id,
    teamAlias: 'platform',
    name: '',
    kind: 'adhoc',
  });
});

test('Create excludes blocked account stores and disables when none remain', () => {
  const blocked: World = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === 'acme' ? { ...server, state: 'blocked' as const } : server,
    ),
  };
  at('?state=create', { world: blocked });
  assert.doesNotMatch(
    text('.sheet'),
    /foks\.acme-corp\.com.*rae\.chen/s,
  );
  assert.match(
    text('.sheet'),
    /foks\.example\.net.*rae/s,
  );
  testingLibrary.cleanup();
  const allBlocked: World = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) => ({
      ...server,
      state: 'blocked' as const,
    })),
  };
  at('?state=groups', { world: allBlocked });
  const create = [
    ...document.querySelectorAll<HTMLButtonElement>('.groups-path button'),
  ].find((button) => button.textContent?.includes('Create group'));
  assert.ok(create?.disabled);
});

test('a Member demotion derives the next lower visibility band from the inspected roster', async () => {
  const world: World = {
    ...FIXTURE,
    parties: FIXTURE.parties.map((party) =>
      party.username === 'priya.n'
        ? {
            ...party,
            source_role: { role: 'Member', visibility: 2 },
            destination_role: { role: 'Member', visibility: 2 },
          }
        : party,
    ),
  };
  const base = mockBridge(world);
  let request: Parameters<typeof base.demoteGroupMember>[0] | null = null;
  at('?state=demote', {
    world,
    bridge: {
      ...base,
      demoteGroupMember: async (value) => {
        request = value;
        return { applied: true };
      },
    },
  });
  assert.match(text('.sheet'), /Member · visibility 2.*Member · visibility 1/);
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button')].find(
      (button) => button.textContent === 'Lower to Member · visibility 1',
    )!,
  );
  await testingLibrary.waitFor(() => assert.ok(request));
  assert.deepEqual(request!.destination, { role: 'Member', visibility: 1 });
});

test('an Owner demotion can choose Admin and sends that strict destination', async () => {
  const world: World = {
    ...FIXTURE,
    parties: FIXTURE.parties.map((party) =>
      party.username === 'priya.n'
        ? {
            ...party,
            source_role: { role: 'Owner' },
            destination_role: { role: 'Owner' },
          }
        : party,
    ),
  };
  const base = mockBridge(world);
  let request: Parameters<typeof base.demoteGroupMember>[0] | null = null;
  at('?state=demote', {
    world,
    bridge: {
      ...base,
      demoteGroupMember: async (value) => {
        request = value;
        return { applied: true };
      },
    },
  });
  const admin = [
    ...document.querySelectorAll<HTMLButtonElement>('.sheet .radio'),
  ].find((button) => button.textContent?.startsWith('Admin'));
  assert.ok(admin);
  testingLibrary.fireEvent.click(admin);
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button')].find(
      (button) => button.textContent === 'Lower to Admin',
    )!,
  );
  await testingLibrary.waitFor(() => assert.ok(request));
  assert.deepEqual(request!.destination, { role: 'Admin' });
});

test('inactive group memberships are struck through and read zero items', () => {
  at('?state=people');
  const row = [...document.querySelectorAll<HTMLElement>('.rt .row')].find(
    (candidate) => candidate.textContent?.includes('homelab'),
  );
  assert.ok(row);
  assert.match(
    row.textContent ?? '',
    /membership inactive · no access here/,
  );
  assert.match(row.querySelector('.strike')?.textContent ?? '', /^0 of /);
});

test('People renders source role and generation facts and labels the fixture machine locally', () => {
  at('?state=people');
  assert.deepEqual(all('.rt .hdr span').slice(0, 7), [
    'Party',
    'Kind',
    'Role here',
    'Role at source',
    'Generation',
    `Reads · of ${FIXTURE.items.filter((item) => item.store === 'team:eng' && item.kind !== 'Folder').length}`,
    'Actions',
  ]);
  const deploy = [...document.querySelectorAll<HTMLElement>('.rt .row')].find(
    (row) => row.textContent?.includes('deploy-bot'),
  );
  assert.ok(deploy);
  assert.match(deploy.textContent ?? '', /machine.*label kept on this Mac/);
  assert.match(deploy.textContent ?? '', /6/);
});

test('Danger offers the safest actionable roster member, derived from authority', () => {
  at('?state=danger');
  const remove = [
    ...document.querySelectorAll<HTMLButtonElement>('.dz button'),
  ].find((button) => button.textContent?.startsWith('Remove '));
  assert.equal(
    remove?.textContent,
    `Remove ${partiesOf(FIXTURE, 'team:eng').find((party) => party.username === 'dana.okafor')?.username}…`,
  );
  assert.ok(remove);
  testingLibrary.fireEvent.click(remove);
  assert.match(text('.sheet h2'), /Remove dana\.okafor from Engineering/);
});

test('Federation compares the retained remote host id with this Mac pin and shows source role', () => {
  at('?state=federation');
  assert.match(text('.fcard'), /matches this Mac’s pin/);
  assert.match(text('.fcard'), /at source: Owner/);
});

test('Federation does not infer that a missing passive host snapshot was never checked', () => {
  const world: World = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) => ({ ...server, host_id: null })),
  };
  at('?state=federation', { world });
  assert.match(text('.fcard'), /local pin fact unavailable/);
  assert.doesNotMatch(text('.fcard'), /never checked/);
});

test('the selected party uses the shell right inspector instead of stacking below the roster', () => {
  at('?state=party');
  assert.ok(document.querySelector('.main > .details'));
  assert.ok(document.querySelector('.body.group-panel-open'));
  assert.match(
    text('.main > .details'),
    /Identity.*What deploy-bot can read.*Connect an agent/,
  );
  const selected = document.querySelector<HTMLElement>('.rt .row.sel');
  assert.ok(selected);
  assert.match(selected.textContent ?? '', /shown in the panel/);
  assert.doesNotMatch(selected.textContent ?? '', /Change role|Remove…/);
  assert.match(text('.main > .details .dfoot'), /Change role.*Remove/);
});

test('roster id Copy does not select the row', async () => {
  let copied = '';
  at('?state=people', {
    bridge: {
      ...mockBridge(FIXTURE),
      copyText: async (value) => {
        copied = value;
        return { ok: true };
      },
    },
  });
  const first = document.querySelector<HTMLElement>('.rt .row');
  const copy = first?.querySelector<HTMLButtonElement>('button');
  assert.ok(first && copy);
  testingLibrary.fireEvent.click(copy);
  await testingLibrary.waitFor(() => assert.ok(copied.length > 14));
  assert.equal(document.querySelector('.main > .details'), null);
});

test('changing groups clears the previous group tab, party and mutation target', async () => {
  at('?state=party');
  assert.match(text('.main > .details'), /deploy-bot/);
  const back = [
    ...document.querySelectorAll<HTMLButtonElement>('.path button'),
  ].find((button) => button.textContent?.includes('Groups'));
  assert.ok(back);
  testingLibrary.fireEvent.click(back);
  const household = [...document.querySelectorAll<HTMLElement>('.gcard')].find(
    (card) => card.textContent?.includes('Household'),
  );
  const open = household?.querySelector<HTMLButtonElement>('button');
  assert.ok(open);
  testingLibrary.fireEvent.click(open);
  assert.equal(document.querySelector('.main > .details'), null);
  await testingLibrary.waitFor(() => {
    assert.match(text('.path'), /Household/);
    assert.equal(document.querySelector('.main > .details'), null);
  });
  assert.match(text('.toolbar .tabs .on'), /People & groups/);
  assert.equal(document.querySelector('.sheet'), null);
});

test('only unique local usernames get group role and removal actions', () => {
  at('?state=people');
  const rows = [...document.querySelectorAll<HTMLElement>('.rt .row')];
  const deploy = rows.find((candidate) =>
    candidate.textContent?.includes('deploy-bot'),
  );
  const self = rows.find((candidate) =>
    candidate.textContent?.includes('rae.chen'),
  );
  const admitted = rows.find((candidate) =>
    candidate.textContent?.includes('homelab'),
  );
  assert.ok(deploy && self && admitted);
  assert.match(deploy.textContent ?? '', /Change role.*Remove/);
  assert.doesNotMatch(self.textContent ?? '', /Remove…/);
  assert.doesNotMatch(admitted.textContent ?? '', /Remove…/);
});

test('duplicate locally manageable usernames fail closed with no role or removal action', () => {
  const dana = FIXTURE.parties.find(
    (party) => party.username === 'dana.okafor',
  );
  assert.ok(dana);
  const world: World = {
    ...FIXTURE,
    parties: [
      ...FIXTURE.parties,
      { ...dana, party_id_hex: `01${'f'.repeat(64)}` },
    ],
  };
  at('?state=people', { world });
  const rows = [...document.querySelectorAll<HTMLElement>('.rt .row')].filter(
    (row) => row.textContent?.includes('dana.okafor'),
  );
  assert.equal(rows.length, 2);
  for (const row of rows) {
    assert.doesNotMatch(row.textContent ?? '', /Change role|Remove…/);
    assert.match(
      row.textContent ?? '',
      /not a unique locally manageable username/,
    );
  }
});

test('the short party removal refuses a non-local service account without invoking Rust', () => {
  let removed = false;
  at('?state=party-remove', {
    bridge: {
      ...mockBridge(FIXTURE),
      removeGroupMember: async () => {
        removed = true;
        return { applied: true };
      },
    },
  });
  const remove = [
    ...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button'),
  ].find((button) => button.textContent === 'Remove and rekey');
  assert.ok(remove);
  assert.ok(remove.disabled);
  assert.match(text('.sheet'), /does not have a unique username managed by this account/);
  assert.ok(document.querySelector('.main .search'));
  assert.match(text('.main'), /production-token/);
  testingLibrary.fireEvent.click(remove);
  assert.equal(removed, false);
});

test('the short Manage scene stays over the Household vault and carries membership facts', () => {
  at('?state=manage');
  assert.ok(document.querySelector('.main .search'));
  assert.match(
    text('.sheet'),
    /id .*People and groups in Household.*foks\.example\.net.*gen 2.*Add people/s,
  );
  assert.match(text('.sheet .vis'), /−Visibility 0\+/);
  const members = document.querySelectorAll('.manage-members .manage-member');
  assert.equal(members.length, partiesOf(FIXTURE, 'team:household').length);
  for (const member of members) {
    assert.ok(member.querySelector('.manage-member-name'));
    assert.ok(member.querySelector('.manage-member-meta'));
    assert.ok(member.querySelector('.manage-member-access'));
  }
  assert.equal(
    [...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button')].at(
      -1,
    )?.textContent,
    'Add',
  );
});

test('Phase 4 group marks and primary action labels match the acceptance surfaces', () => {
  at('?state=groups');
  assert.equal(document.querySelectorAll('.gcard .kico.group').length, 3);
  testingLibrary.cleanup();
  at('?state=groups-lease');
  assert.match(text('.path'), /‹ Groups.*Engineering/);
  assert.ok(document.querySelector('.path .kico.group'));
  testingLibrary.cleanup();
  at('?state=groups-inactive');
  assert.match(text('.path'), /‹ Groups.*Homelab/);
  assert.ok(document.querySelector('.path .kico.group'));
  testingLibrary.cleanup();
  at('?state=add');
  assert.equal(
    [...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button')].at(
      -1,
    )?.textContent,
    'Add as Member · visibility 0',
  );
  testingLibrary.cleanup();
  at('?state=admit');
  assert.equal(
    [...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button')].at(
      -1,
    )?.textContent,
    'Add as Member · visibility 0',
  );
  testingLibrary.cleanup();
  at('?state=create');
  assert.doesNotMatch(text('.sheet'), /account there is unavailable/);
});

test('Manage on a group vault navigates and opens that group management overlay', async () => {
  at('?state=group');
  const manage = [
    ...document.querySelectorAll<HTMLButtonElement>('.path button'),
  ].find((button) => button.textContent === 'Manage');
  assert.ok(manage);
  testingLibrary.fireEvent.click(manage);
  await testingLibrary.waitFor(() =>
    assert.match(text('.sheet h2'), /Manage Household/),
  );
  assert.match(text('.sheet'), /Reads \d+ of \d+/);
});

test('active ad-hoc groups expose roster facts but no management controls', () => {
  const world: World = {
    ...FIXTURE,
    stores: FIXTURE.stores.map((store) =>
      store.id === 'team:homelab' && store.kind === 'team'
        ? { ...store, active: true }
        : store,
    ),
  };
  at('?state=group-admin&store=team%3Ahomelab', { world });
  assert.match(
    text('.groups-wrap'),
    /changes are unavailable.*read-only roster facts/i,
  );
  assert.equal(document.querySelector('.groups-wrap > .band'), null);
  assert.ok(document.querySelector('.groups-wrap > .group-limit'));
  const invite = [
    ...document.querySelectorAll<HTMLButtonElement>('.toolbar button'),
  ].find((button) => button.textContent === 'Invite someone');
  const add = [
    ...document.querySelectorAll<HTMLButtonElement>('.groups-wrap button'),
  ].find((button) => button.textContent === 'Add someone');
  assert.ok(invite?.disabled);
  assert.ok(add?.disabled);
  testingLibrary.cleanup();
  at('?state=homelab', { world });
  const manage = [
    ...document.querySelectorAll<HTMLButtonElement>('.path button'),
  ].find((button) => button.textContent === 'Manage');
  assert.ok(manage?.disabled);
});

test('a truly lapsed native-style world stays stopped on the ordinary People deep link', () => {
  const world = applyLease(FIXTURE, 'lapsed');
  at('?state=people', { world });
  assert.ok(document.querySelector('.notice'));
  assert.equal(document.querySelector('.rt'), null);
});

test('the Groups list never turns a suppressed lapsed roster into a zero-member fact', () => {
  at('?state=groups', { world: applyLease(FIXTURE, 'lapsed') });
  const engineering = [
    ...document.querySelectorAll<HTMLElement>('.gcard'),
  ].find((card) => card.textContent?.includes('Engineering'));
  assert.ok(engineering);
  assert.match(engineering.textContent ?? '', /Roster unavailable/);
  assert.doesNotMatch(engineering.textContent ?? '', /\d+ people/);
});

test('an ambiguous group resumable refreshes once and is never replayed', async () => {
  const base = mockBridge(FIXTURE);
  let resumes = 0;
  let catalogs = 0;
  at('?state=groups-inactive', {
    bridge: {
      ...base,
      listCatalog: async () => {
        catalogs += 1;
        return base.listCatalog();
      },
      resumeGroupCreation: async () => {
        resumes += 1;
        throw {
          code: 'ambiguous',
          message: 'The group resume outcome is uncertain.',
          retryable: false,
          ambiguous: true,
          fatal: false,
        };
      },
    },
  });
  const resume = document.querySelector<HTMLButtonElement>('.notice button');
  assert.ok(resume);
  testingLibrary.fireEvent.click(resume);
  await testingLibrary.waitFor(() =>
    assert.match(text('.flash'), /outcome is uncertain.*State refreshed/i),
  );
  assert.equal(resumes, 1);
  assert.equal(catalogs, 1);
});

test('group member add reloads and the stateful mock exposes the new roster entry', async () => {
  at('?state=add');
  const input = document.querySelector<HTMLInputElement>('.sheet input');
  assert.ok(input);
  testingLibrary.fireEvent.change(input, { target: { value: 'new.person' } });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button')].find(
      (button) => button.textContent === 'Add as Member · visibility 0',
    )!,
  );
  await testingLibrary.waitFor(() => assert.match(text('.flash'), /completed/));
  assert.match(text('.rt'), /new.person/);
});

const FIRST_RUN_RENDER_STATES = [
  'boot',
  'who',
  'address',
  'no-address',
  'checked',
  'compare',
  'error',
  'account',
  'existing',
  'protect',
  'phrase',
  'waiting',
  'added',
  'create-group',
  'done',
  'checklist-invited',
  'checklist-own',
] as const;

const FIRST_RUN_COPY: Record<(typeof FIRST_RUN_RENDER_STATES)[number], RegExp> =
  {
    boot: /Preparing this Mac/,
    who: /Who is setting you up/,
    address: /What did sam send you|Which server/,
    'no-address': /Ask .* this|Which server/,
    checked: /Found it/,
    compare: /Published id/,
    error: /nothing was saved|Nothing was checked/i,
    account: /Your account on/,
    existing: /Add this Mac to your account/,
    protect: /Right now this Mac holds the only key/,
    phrase: /Write these tokens down/,
    waiting: /Waiting for/,
    added: /You’re in/,
    'create-group': /Your first group/,
    done: /exists/,
    'checklist-invited': /Get started/,
    'checklist-own': /Get started/,
  };

for (const path of ['invited', 'own'] as const) {
  for (const state of FIRST_RUN_RENDER_STATES) {
    test(`Phase 5 ${path} first-run state ${state} renders in the shared shell`, async () => {
      const effectivePath =
        state === 'waiting' || state === 'added' || state === 'checklist-invited'
          ? 'invited'
          : state === 'create-group' || state === 'done' || state === 'checklist-own'
            ? 'own'
            : path;
      at(`?state=${state}&path=${path}`);
      await testingLibrary.waitFor(() =>
        assert.match(
          state === 'phrase'
            ? (document.body.textContent ?? '')
            : text('.window'),
          FIRST_RUN_COPY[state],
        ),
      );
      assert.ok(document.querySelector('.app .side'));
      assert.ok(document.querySelector('.app .main'));
      assert.equal(document.querySelector('.setup-side .slabel'), null);
      const footerNote = document.querySelector<HTMLElement>(
        '.first-run-main .pfoot .note',
      );
      if (footerNote) {
        const action = footerNote.querySelector('button');
        assert.ok(action, 'footer notes contain actions, not guide text');
        assert.equal(footerNote.textContent?.trim(), action.textContent?.trim());
      }
      if (!['added', 'done', 'checklist-invited', 'checklist-own'].includes(state)) {
        assert.equal(
          document.querySelector('.first-run-main > .path'),
          null,
          'setup content begins directly with the pane',
        );
        assert.equal(document.querySelector('.setup-step small'), null);
        assert.equal(document.querySelector('.setup-note'), null);
        assert.doesNotMatch(text('.side'), /FOKS first run|Stop any time/);
      }
      if (state === 'boot') {
        assert.match(text('.window'), /Agent starting/);
        assert.doesNotMatch(text('.window'), /Agent ready/);
        assert.match(
          text('.first-run-main'),
          /Starting a local foks-agent and initializing an encrypted store for your keys/,
        );
        assert.match(
          text('.first-run-main'),
          /Starting macOS private helper.*bound to this window.*Creating encrypted vault.*end-to-end encrypted vault.*Continue to the next step/s,
        );
      }
      if (
        [
          'checked', 'compare', 'account', 'existing', 'waiting', 'added',
          'create-group', 'done', 'checklist-invited',
          'checklist-own',
        ].includes(state)
      ) {
        const facts = mockBridge(FIXTURE).firstRunFixture?.[effectivePath];
        assert.ok(facts);
        assert.match(
          document.body.textContent ?? '',
          new RegExp(facts.report.canonicalName),
        );
      }
      if (state === 'phrase') {
        await testingLibrary.waitFor(() =>
          assert.equal(document.querySelectorAll('.sheet .word').length, 17),
        );
        assert.equal(
          [
            ...document.querySelectorAll<HTMLButtonElement>('.sheet button'),
          ].some((button) => /copy/i.test(button.textContent ?? '')),
          false,
        );
      }
      if (state === 'existing') {
        const cards = [...document.querySelectorAll<HTMLElement>('.pcard')];
        assert.match(cards[0]?.textContent ?? '', /Recover with your backup phrase/);
        assert.doesNotMatch(cards[0]?.textContent ?? '', /Works today|Requires|interrupted/);
        assert.match(cards[1]?.textContent ?? '', /Pair from a Mac you already use/);
        assert.equal(
          cards[1]?.querySelector('p')?.textContent?.trim(),
          'On another signed-in Mac, open Settings › Your Macs & recovery and choose Add another Mac.',
        );
        assert.equal(cards[1]?.querySelector('p b'), null);
        assert.doesNotMatch(cards[1]?.textContent ?? '', /available now|Requires/);
        const pairingButtons = cards[1]?.querySelectorAll<HTMLButtonElement>('button');
        assert.equal(pairingButtons?.[0]?.disabled, true, 'Accept waits for the secret phrase');
        assert.equal(pairingButtons?.[1]?.disabled, false, 'Resume needs only the authenticated pending identity');
      }
      if (state === 'added') {
        assert.doesNotMatch(document.body.textContent ?? '', /\[object Object\]/);
        if (effectivePath === 'invited') {
          const bundle = [
            ...document.querySelectorAll<HTMLElement>('.first-run-main .row'),
          ].find((row) => row.textContent?.includes('bundle.tar'));
          assert.ok(bundle?.querySelector('.kic.File svg'));
        }
      }
      if (state === 'checked') {
        const checked = text('.first-run-main');
        const facts = mockBridge(FIXTURE).firstRunFixture?.[effectivePath];
        assert.ok(facts);
        assert.match(checked, new RegExp(facts.report.canonicalName));
        assert.match(
          text('.first-run-main details'),
          new RegExp(facts.report.hostId.slice(0, 4)),
        );
        assert.match(checked, /Pinned on this Mac/);
        assert.match(checked, /compatibility lease.*current/s);
        assert.match(checked, /Server check result.*New pin.*Same as before.*Advanced/s);
        assert.match(
          checked,
          effectivePath === 'invited'
            ? /Compare with a host id sam published/
            : /Compare with a host id your server’s operator published/,
        );
        assert.match(
          checked,
          /Optional.*Paste the published host ID.*comparison is optional and runs only on this Mac.*does not change the pinned host ID/s,
        );
        assert.equal(
          document.querySelector<HTMLInputElement>(
            '.first-run-main .inset input',
          )?.value,
          path === 'invited' ? 'foks.acme-corp.com' : 'foks.example.net',
        );
        assert.ok(
          [...document.querySelectorAll<HTMLButtonElement>('button')].some(
            (button) => button.textContent === 'Check again',
          ),
        );
        assert.ok(document.querySelector('.first-run-main .pcard h3 svg'));
        assert.doesNotMatch(text('.first-run-main'), /phase/i);
      }
      if (state === 'who') {
        const introduction = document.querySelector<HTMLElement>(
          '.first-run-main .lead',
        );
        assert.ok(introduction);
        assert.match(
          introduction.textContent ?? '',
          /encrypted stores.*your own in Personal, shared ones in groups/s,
        );
        assert.doesNotMatch(introduction.textContent ?? '', /shared store with a roster/);
        assert.equal(
          [...introduction.querySelectorAll('b')].some(
            (element) => element.textContent === 'stores',
          ),
          false,
        );
        assert.match(text('.first-run-main'), /This Mac is added to your devices/);
      }
      if (state === 'account') {
        assert.match(
          text('.first-run-main'),
          /Letters, digits and dots.*rename it any time/s,
        );
      }
      if (
        state === 'protect' ||
        state === 'phrase' ||
        state.startsWith('checklist-')
      ) {
        assert.doesNotMatch(
          document.body.textContent ?? '',
          /17 words|17-word/i,
        );
      }
      if (state === 'waiting') {
        assert.match(
          text('.first-run-main .band'),
          /Group discovery.*Check now.*authenticated account/s,
        );
        const waiting = text('.first-run-main');
        assert.match(
          waiting,
          /FOKS lists groups after it checks the server/s,
        );
        assert.match(
          waiting,
          /visibility level.*above your role or visibility level remain locked/s,
        );
        assert.match(
          waiting,
          /Not checked yet.*Checks stop when FOKS is closed/s,
        );
        assert.match(waiting, /Use your Personal vault/);
        assert.match(waiting, /confirm that they entered/);
      }
      if (state === 'create-group') {
        assert.match(
          text('.first-run-main'),
          /Add people by username from Manage.*select Resume/s,
        );
      }
      if (state.startsWith('checklist-')) {
        assert.match(text('.first-run-main'), /Get started3 of 5 done/);
        assert.match(
          text('.first-run-main'),
          /Continue the remaining setup steps/,
        );
        if (effectivePath === 'invited') {
          assert.match(
            text('.first-run-main'),
            /Copy the sentence.*Check now/s,
          );
          assert.match(
            text('.first-run-main .band'),
            /Group discovery.*authenticated account/s,
          );
        }
      }
    });
  }
}

test('the own-server prompt omits the extra explanation and no-server help', () => {
  at('?state=address&path=own');
  const main = document.querySelector<HTMLElement>('.first-run-main');
  assert.ok(main);
  assert.equal(main.querySelector('.lead b'), null);
  assert.doesNotMatch(
    main.textContent ?? '',
    /web address without|If someone else runs it|server holds ciphertext|Don’t have one/,
  );
  assert.equal(main.querySelector('.pcard'), null);
  const address = main.querySelector<HTMLInputElement>(
    'input[aria-label="Server address"]',
  );
  assert.ok(address);
  assert.ok(
    address.closest('label.server-address-row'),
    'the whole address row labels and focuses its input',
  );
});

test('the one-time backup reveal does not issue concurrent prepares in StrictMode', async () => {
  window.history.replaceState(null, '', '/?state=phrase&path=own');
  const base = mockBridge(FIXTURE);
  let prepares = 0;
  const bridge = {
    ...base,
    prepareOwnerBackup: async (
      profile: string,
      accountAlias: string,
      backupAlias: string,
    ) => {
      prepares += 1;
      return base.prepareOwnerBackup(profile, accountAlias, backupAlias);
    },
  };
  testingLibrary.render(
    createElement(
      StrictMode,
      null,
      createElement(App, { world: FIXTURE, bridge }),
    ),
  );
  await testingLibrary.waitFor(() =>
    assert.equal(document.querySelectorAll('.sheet .word').length, 17),
  );
  assert.equal(prepares, 1);
});

test('the Sol flow uses authenticated discovery and survives reload checkpoints', async () => {
  window.localStorage.removeItem('foks.first-run.v1');
  at('?state=who&path=invited');
  const choiceCards = [...document.querySelectorAll<HTMLElement>('.choice-card')];
  assert.equal(choiceCards.length, 3);
  assert.ok(choiceCards.every((card) => card.tagName === 'ARTICLE'));
  assert.ok(choiceCards.every((card) => card.querySelector('ol')));
  assert.ok(choiceCards.every((card) => card.querySelector('button.primary')));
  const invited = [...document.querySelectorAll<HTMLButtonElement>('.choice-card button')]
    .find((button) => button.textContent === 'Start here');
  assert.ok(invited);
  testingLibrary.fireEvent.click(invited);
  await testingLibrary.waitFor(() =>
    assert.match(text('.pane h1'), /What did sam send you/),
  );
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pfoot button')].find(
      (button) => button.textContent === 'Check the server',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.pane h1'), /Found it/),
  );
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pfoot button')].find(
      (button) => button.textContent === 'Continue',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.pane h1'), /Your account on/),
  );
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pfoot button')].find(
      (button) => button.textContent === 'Create my account',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.pane h1'), /Protect it/),
  );
  const saved = window.localStorage.getItem('foks.first-run.v1') ?? '';
  assert.match(saved, /"username":"sol"/);
  assert.doesNotMatch(
    saved,
    /"invite"|"passphrase"|"recoveryPhrase"|"backupPhrase"/,
  );

  testingLibrary.cleanup();
  window.history.replaceState(
    null,
    '',
    '/?state=first-run&step=protect&path=invited',
  );
  testingLibrary.render(
    createElement(App, { world: FIXTURE, bridge: mockBridge(FIXTURE) }),
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /only key to.*sol/s),
  );
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pfoot .lnk')].find(
      (button) => /Skip for now/.test(button.textContent ?? ''),
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /Get started/),
  );
  const checkNow = [
    ...document.querySelectorAll<HTMLButtonElement>('.main button'),
  ].find((button) => button.textContent === 'Check now');
  assert.ok(checkNow);
  testingLibrary.fireEvent.click(checkNow);
  await testingLibrary.waitFor(() =>
    assert.match(text('.pane h1'), /Waiting for sam/),
  );
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pcard button')].find(
      (button) => button.textContent === 'Check now',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /You’re in Engineering/),
  );
});

test('Set up again starts a clean first-run flow', async () => {
  window.localStorage.removeItem('foks.first-run.v1');
  at('?state=done&path=own');
  assert.match(text('.main'), /Household exists/);
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.side button')].find(
      (button) => button.textContent?.includes('Set up again'),
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /Who is setting you up/),
  );
  const cancel = [
    ...document.querySelectorAll<HTMLButtonElement>('.side .foot button'),
  ].find((button) => button.textContent?.includes('Cancel setup'));
  assert.ok(cancel);
  testingLibrary.fireEvent.click(cancel);
  await testingLibrary.waitFor(() =>
    assert.equal(text('.main .loc h1'), 'All items'),
  );
  const after = decodeFirstRunCheckpoint(
    window.localStorage.getItem('foks.first-run.v1'),
  );
  assert.equal(after?.state, 'who');
  assert.equal(after?.profile, undefined);
  assert.equal(after?.account, undefined);
  assert.equal(after?.group, undefined);
});

test('Settings uses a cog rather than the radial sun glyph', () => {
  const gear = FOKS_ICONS.gear;
  assert.equal(gear.length, 2);
  assert.equal(gear[0]?.[0], 'path');
  assert.doesNotMatch(String(gear[0]?.[1].d), /M12 2v3/);
  assert.equal(gear[1]?.[0], 'circle');
});

test('native recovery takes an explicit alias and refreshes the authenticated username', async () => {
  const profile = {
    profile: 'personal',
    acceptance: 'inserted' as const,
    lookupName: 'foks.example.net',
    canonicalName: 'foks.example.net',
    hostId: `02${'8'.repeat(64)}`,
    chain: 4,
    epoch: 10,
  };
  window.localStorage.setItem(
    'foks.first-run.v1',
    encodeFirstRunCheckpoint({
      ...initialFirstRun('own', 'existing'),
      initialized: true,
      returning: true,
      profile,
      serverAddress: 'foks.example.net',
    }),
  );
  const base = mockBridge(FIXTURE);
  let request:
    | {
        profile: string;
        targetAlias: string;
        phrase: string;
        deviceName: string;
      }
    | undefined;
  const bridge = {
    ...nativeAvailableBridge(base),
    recoverOwnerAccount: async (
      server: string,
      alias: string,
      phrase: string,
      deviceName: string,
    ) => {
      request = { profile: server, targetAlias: alias, phrase, deviceName };
      return base.recoverOwnerAccount(server, alias, phrase, deviceName);
    },
  };
  at('?state=first-run&step=existing&path=own', { bridge });
  const alias = document.querySelector<HTMLInputElement>(
    'input:not([type="password"])',
  );
  const phrase = document.querySelector<HTMLInputElement>(
    'input[aria-label="Backup phrase"]',
  );
  const device = [...document.querySelectorAll<HTMLInputElement>('input')].find(
    (input) => input !== alias && input !== phrase,
  );
  assert.ok(alias && phrase && device);
  testingLibrary.fireEvent.change(alias, { target: { value: 'personal' } });
  testingLibrary.fireEvent.change(phrase, {
    target: { value: 'seventeen private tokens' },
  });
  testingLibrary.fireEvent.change(device, { target: { value: 'New Mac' } });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pcard button')].find(
      (button) => button.textContent === 'Recover',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /only key to.*rae/s),
  );
  assert.deepEqual(request, {
    profile: 'personal',
    targetAlias: 'personal',
    phrase: 'seventeen private tokens',
    deviceName: 'New Mac',
  });
});

test('a new Mac can accept pairing from first run before it has an account store', async () => {
  const profile = {
    profile: 'personal',
    acceptance: 'inserted' as const,
    lookupName: 'foks.example.net',
    canonicalName: 'foks.example.net',
    hostId: `02${'8'.repeat(64)}`,
    chain: 4,
    epoch: 10,
  };
  window.localStorage.setItem(
    'foks.first-run.v1',
    encodeFirstRunCheckpoint({
      ...initialFirstRun('own', 'existing'),
      initialized: true,
      returning: true,
      profile,
      serverAddress: 'foks.example.net',
    }),
  );
  const base = mockBridge(FIXTURE);
  let paired = false;
  let accepted: { profile: string; alias: string; device: string; phrase: string } | undefined;
  const bridge: Bridge = {
    ...nativeAvailableBridge(base),
    listCatalog: async () => {
      const catalog = await base.listCatalog();
      return {
        ...catalog,
        profiles: ['personal'],
        stores: paired ? catalog.stores.filter((store) => store.id === 'acct:personal') : [],
        items: paired ? catalog.items.filter((item) => item.store === 'acct:personal') : [],
        failures: [],
        blockedProfiles: [],
      };
    },
    listServers: async () => (await base.listServers()).filter((server) => server.id === 'personal'),
    listAccounts: async () => paired
      ? (await base.listAccounts()).filter((account) => account.store === 'acct:personal')
      : [],
    listParties: async () => [],
    listFederation: async () => [],
    acceptDevicePairing: async (server, alias, device, phrase) => {
      accepted = { profile: server, alias, device, phrase };
      paired = true;
      return base.acceptDevicePairing(server, alias, device, phrase);
    },
  };
  const personalServer = FIXTURE.servers.find((server) => server.id === 'personal');
  assert.ok(personalServer);
  at('?state=existing&path=own', {
    bridge,
    world: {
      ...FIXTURE,
      stores: [],
      items: [],
      accounts: [],
      parties: [],
      federation: [],
      servers: [personalServer],
      notifications: [],
    },
  });
  testingLibrary.fireEvent.change(
    document.querySelector<HTMLInputElement>('[aria-label="Pairing account alias"]')!,
    { target: { value: 'personal' } },
  );
  testingLibrary.fireEvent.change(
    document.querySelector<HTMLInputElement>('[aria-label="Pairing device name"]')!,
    { target: { value: 'New Mac' } },
  );
  const phrase = document.querySelector<HTMLInputElement>('[aria-label="Pairing phrase"]');
  assert.ok(phrase);
  testingLibrary.fireEvent.change(phrase, { target: { value: 'cobalt window' } });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pcard button')].find(
      (button) => button.textContent === 'Accept pairing',
    )!,
  );
  assert.equal(phrase.value, '', 'the phrase is cleared before awaiting the command');
  await testingLibrary.waitFor(() => assert.match(text('.main'), /Protect it/));
  assert.deepEqual(accepted, {
    profile: 'personal', alias: 'personal', device: 'New Mac', phrase: 'cobalt window',
  });
  assert.doesNotMatch(window.localStorage.getItem('foks.first-run.v1') ?? '', /cobalt window/);
});

test('native discovery carries the authenticated group identity and refreshes before drawing it', async () => {
  const teamIdHex = `03${'7'.repeat(64)}`;
  const nativeWorld: World = {
    ...FIXTURE,
    stores: FIXTURE.stores.map((store) =>
      store.id === 'team:eng'
        ? {
            ...store,
            name: 'Live Ops',
            alias: 'live-ops',
            account: 'sol',
            team_id_hex: teamIdHex,
          }
        : store,
    ),
  };
  const profile = {
    profile: 'acme',
    acceptance: 'inserted' as const,
    lookupName: 'foks.acme-corp.com',
    canonicalName: 'foks.acme-corp.com',
    hostId: `02${'8'.repeat(64)}`,
    chain: 4,
    epoch: 10,
  };
  window.localStorage.setItem(
    'foks.first-run.v1',
    encodeFirstRunCheckpoint({
      ...initialFirstRun('invited', 'waiting'),
      initialized: true,
      profile,
      serverAddress: 'foks.acme-corp.com',
      account: { alias: 'sol', username: 'sol', deviceName: "Sol's Mac" },
      passphraseSet: true,
    }),
  );
  const base = mockBridge(nativeWorld);
  const calls: string[] = [];
  const bridge = {
    ...nativeAvailableBridge(base),
    discoverGroups: async () => {
      calls.push('discover');
      return {
        accountAlias: 'sol',
        groups: [
          {
            alias: 'live-ops',
            accountAlias: 'sol',
            teamIdHex,
            kind: 'named' as const,
            name: 'Live Ops',
            active: true,
          },
        ],
      };
    },
    listCatalog: async () => {
      calls.push('catalog');
      return base.listCatalog();
    },
  };
  at('?state=first-run&step=waiting&path=invited', {
    world: nativeWorld,
    bridge,
  });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pcard button')].find(
      (button) => button.textContent === 'Check now',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /You’re in Live Ops/),
  );
  assert.doesNotMatch(text('.main'), /You’re in Engineering/);
  assert.match(text('.main'), /Authenticated discovery now lists sol/);
  assert.doesNotMatch(text('.main'), /added sol as a Member/);
  assert.deepEqual(calls.slice(0, 2), ['discover', 'catalog']);
});

test('native discovery rejects a refreshed group owned by another account', async () => {
  const teamIdHex = `03${'6'.repeat(64)}`;
  const wrongAccountWorld: World = {
    ...FIXTURE,
    stores: FIXTURE.stores.map((store) =>
      store.id === 'team:eng'
        ? {
            ...store,
            name: 'Live Ops',
            alias: 'live-ops',
            server: 'acme',
            account: 'someone-else',
            team_id_hex: teamIdHex,
          }
        : store,
    ),
  };
  window.localStorage.setItem(
    'foks.first-run.v1',
    encodeFirstRunCheckpoint({
      ...initialFirstRun('invited', 'waiting'),
      initialized: true,
      profile: {
        profile: 'acme',
        acceptance: 'inserted',
        lookupName: 'foks.acme-corp.com',
        canonicalName: 'foks.acme-corp.com',
        hostId: `02${'5'.repeat(64)}`,
        chain: 4,
        epoch: 10,
      },
      serverAddress: 'foks.acme-corp.com',
      account: { alias: 'sol', username: 'sol', deviceName: "Sol's Mac" },
      passphraseSet: true,
    }),
  );
  const base = mockBridge(wrongAccountWorld);
  const bridge = {
    ...nativeAvailableBridge(base),
    discoverGroups: async () => ({
      accountAlias: 'sol',
      groups: [
        {
          alias: 'live-ops',
          accountAlias: 'sol',
          teamIdHex,
          kind: 'named' as const,
          name: 'Live Ops',
          active: true,
        },
      ],
    }),
  };
  at('?state=first-run&step=waiting&path=invited', {
    world: wrongAccountWorld,
    bridge,
  });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pcard button')].find(
      (button) => button.textContent === 'Check now',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(
      text('.main'),
      /authenticated group was found, but its refreshed store is not available yet/i,
    ),
  );
  assert.doesNotMatch(text('.main'), /You’re in Live Ops/);
});

test('native discovery rejects a response bound to another account before refresh', async () => {
  const teamIdHex = `03${'6'.repeat(64)}`;
  window.localStorage.setItem(
    'foks.first-run.v1',
    encodeFirstRunCheckpoint({
      ...initialFirstRun('invited', 'waiting'),
      initialized: true,
      profile: {
        profile: 'acme',
        acceptance: 'inserted',
        lookupName: 'foks.acme-corp.com',
        canonicalName: 'foks.acme-corp.com',
        hostId: `02${'5'.repeat(64)}`,
        chain: 4,
        epoch: 10,
      },
      serverAddress: 'foks.acme-corp.com',
      account: { alias: 'sol', username: 'sol', deviceName: "Sol's Mac" },
      passphraseSet: true,
    }),
  );
  const base = mockBridge(FIXTURE);
  let refreshes = 0;
  const bridge = {
    ...nativeAvailableBridge(base),
    discoverGroups: async () => ({
      accountAlias: 'someone-else',
      groups: [
        {
          alias: 'live-ops',
          accountAlias: 'someone-else',
          teamIdHex,
          kind: 'named' as const,
          name: 'Live Ops',
          active: true,
        },
      ],
    }),
    listCatalog: async () => {
      refreshes += 1;
      return base.listCatalog();
    },
  };
  at('?state=first-run&step=waiting&path=invited', {
    world: FIXTURE,
    bridge,
  });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pcard button')].find(
      (button) => button.textContent === 'Check now',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /belongs to a different account/i),
  );
  assert.equal(refreshes, 0);
  assert.doesNotMatch(text('.main'), /You’re in Live Ops/);
});

test('own-path group creation establishes a catalog generation before reading accounts', async () => {
  const profile = {
    profile: 'personal',
    acceptance: 'inserted' as const,
    lookupName: 'foks.example.net',
    canonicalName: 'foks.example.net',
    hostId: `02${'9'.repeat(64)}`,
    chain: 2,
    epoch: 3,
  };
  window.localStorage.setItem(
    'foks.first-run.v1',
    encodeFirstRunCheckpoint({
      ...initialFirstRun('own', 'create-group'),
      initialized: true,
      profile,
      serverAddress: 'foks.example.net',
      account: { alias: 'personal', username: 'rae', deviceName: 'MacBook' },
      backupCommitted: true,
    }),
  );
  const freshWorld: World = {
    ...FIXTURE,
    stores: FIXTURE.stores.filter((store) => store.id !== 'team:household'),
    items: FIXTURE.items.filter((item) => item.store !== 'team:household'),
    parties: FIXTURE.parties.filter(
      (party) => party.store !== 'team:household',
    ),
    federation: FIXTURE.federation.filter(
      (entry) => entry.store !== 'team:household',
    ),
  };
  const base = mockBridge(freshWorld);
  const calls: string[] = [];
  const bridge = {
    ...nativeAvailableBridge(base),
    listCatalog: async () => {
      calls.push('catalog');
      return base.listCatalog();
    },
    listAccounts: async () => {
      calls.push('accounts');
      const length = calls.length;
      assert.equal(calls[length - 2], 'catalog');
      return base.listAccounts();
    },
    createGroup: async (request: Parameters<typeof base.createGroup>[0]) => {
      calls.push(`create:${request.teamAlias}`);
      return base.createGroup(request);
    },
  };
  at('?state=first-run&step=create-group&path=own', {
    world: freshWorld,
    bridge,
  });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pfoot button')].find(
      (button) => button.textContent === 'Create group',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /Household exists/),
  );
  await testingLibrary.act(async () => Promise.resolve());
  assert.deepEqual(calls, [
    'catalog',
    'accounts',
    'create:household',
    'catalog',
    'accounts',
  ]);
  const saved = decodeFirstRunCheckpoint(
    window.localStorage.getItem('foks.first-run.v1'),
  );
  assert.deepEqual(saved?.group, {
    name: 'Household',
    kind: 'named',
    alias: 'household',
    teamIdHex: `03${'1'.repeat(64)}`,
  });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('button')].find(
      (button) => button.textContent === 'Open your vault',
    )!,
  );
  await testingLibrary.waitFor(() => {
    const search = new URLSearchParams(window.location.search);
    assert.equal(search.get('state'), 'store');
    assert.equal(search.get('store'), 'team:household');
  });
});

test('own-path group creation refuses success until refresh binds one authenticated active team', async () => {
  const profile = {
    profile: 'personal',
    acceptance: 'inserted' as const,
    lookupName: 'foks.example.net',
    canonicalName: 'foks.example.net',
    hostId: `02${'9'.repeat(64)}`,
    chain: 2,
    epoch: 3,
  };
  window.localStorage.setItem(
    'foks.first-run.v1',
    encodeFirstRunCheckpoint({
      ...initialFirstRun('own', 'create-group'),
      profile,
      serverAddress: 'foks.example.net',
      account: { alias: 'personal', username: 'rae', deviceName: 'MacBook' },
    }),
  );
  const world: World = {
    ...FIXTURE,
    stores: FIXTURE.stores.filter((store) => store.id !== 'team:household'),
    items: FIXTURE.items.filter((item) => item.store !== 'team:household'),
    parties: FIXTURE.parties.filter(
      (party) => party.store !== 'team:household',
    ),
  };
  const base = mockBridge(world);
  const bridge = {
    ...nativeAvailableBridge(base),
    createGroup: async () => ({ applied: true as const }),
  };
  at('?state=first-run&step=create-group&path=own', { world, bridge });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pfoot button')].find(
      (button) => button.textContent === 'Create group',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(
      text('.main'),
      /authenticated active store is not available after refresh/,
    ),
  );
  assert.match(text('.main'), /Create a group/);
  assert.equal(
    decodeFirstRunCheckpoint(
      window.localStorage.getItem('foks.first-run.v1'),
    )?.state,
    'create-group',
  );
});

test('agent loss clears every first-run secret and closes the prepared phrase', async () => {
  const base = mockBridge(FIXTURE);
  let reportLoss: ((message: string | null) => void) | undefined;
  const bridge = {
    ...base,
    takeAgentConnectionLoss: () =>
      new Promise<string | null>((resolve) => {
        reportLoss = resolve;
      }),
  };
  at('?state=protect&path=invited', { bridge });
  // Settle the authenticated resumables read started by the first-run mount
  // before driving the independent connection-loss signal.
  await testingLibrary.act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
  const passphrase = document.querySelector<HTMLInputElement>(
    'input[aria-label="Passphrase"]',
  );
  assert.ok(passphrase);
  testingLibrary.fireEvent.change(passphrase, {
    target: { value: 'never-retain-this' },
  });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('button')].find(
      (button) => button.textContent === 'Show my phrase',
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.equal(document.querySelectorAll('.sheet .word').length, 17),
  );
  await testingLibrary.act(async () => {
    reportLoss?.('transport closed');
    await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  await testingLibrary.waitFor(() => {
    assert.match(text('.stopwrap'), /Your data has been saved/);
    assert.doesNotMatch(text('.stopwrap'), /transport closed/);
  });
  await testingLibrary.waitFor(() =>
    assert.equal(
      new URLSearchParams(window.location.search).get('step'),
      'protect',
    ),
  );
  assert.equal(document.querySelector('.sheet'), null);
  assert.equal(passphrase.value, '');
  assert.doesNotMatch(document.body.textContent ?? '', /orbit velvet lantern/);
});

test('Protect refuses mismatched passphrase confirmation before a command', () => {
  at('?state=protect&path=invited');
  const inputs = [
    ...document.querySelectorAll<HTMLInputElement>('input[type="password"]'),
  ];
  testingLibrary.fireEvent.change(inputs[0], {
    target: { value: 'one value' },
  });
  testingLibrary.fireEvent.change(inputs[1], {
    target: { value: 'another value' },
  });
  const button = [
    ...document.querySelectorAll<HTMLButtonElement>('.pfoot button'),
  ].find((candidate) => candidate.textContent === 'Continue');
  assert.ok(button?.disabled);
});

test('intentionally skipping protection clears its secret draft before review', async () => {
  at('?state=protect&path=invited');
  const input = document.querySelector<HTMLInputElement>(
    'input[aria-label="Passphrase"]',
  );
  assert.ok(input);
  testingLibrary.fireEvent.change(input, {
    target: { value: 'do-not-retain-this' },
  });
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.pfoot button')].find(
      (button) => button.textContent?.includes('Skip for now'),
    )!,
  );
  await testingLibrary.waitFor(() =>
    assert.match(text('.main'), /Get started/),
  );
  testingLibrary.fireEvent.click(
    [...document.querySelectorAll<HTMLButtonElement>('.main button')].find(
      (button) => button.textContent === 'Protect now',
    )!,
  );
  await testingLibrary.waitFor(() => {
    const revisited = document.querySelector<HTMLInputElement>(
      'input[aria-label="Passphrase"]',
    );
    assert.equal(revisited?.value, '');
  });
});

/* ---------------------------------------------------------------- search -- */

test('typing in the search field filters and keeps the caret', async () => {
  at('?state=all');
  const input = document.querySelector<HTMLInputElement>('.search input');
  assert.ok(input);
  input.focus();

  testingLibrary.fireEvent.change(input, { target: { value: 'wifi' } });
  await testingLibrary.waitFor(() => {
    assert.equal(document.querySelectorAll('.body .row').length, 1);
  });

  // The same node, still focused, still holding what was typed: the whole
  // reason the render layer is React and not `innerHTML`.
  const after = document.querySelector<HTMLInputElement>('.search input');
  assert.equal(after, input, 'the input survived the re-render');
  assert.equal(document.activeElement, input);
  assert.equal(input.value, 'wifi');
  assert.equal(
    row('guest-password').querySelector('.name small code')?.textContent,
    '/wifi/guest-password',
  );

  testingLibrary.fireEvent.change(input, { target: { value: 'nothing here' } });
  await testingLibrary.waitFor(() => {
    assert.match(text('.empty h2'), /matches “nothing here”/);
  });
  assert.equal(document.activeElement, input);
});

/* ------------------------------------------------------------ the toolbar -- */

test('the New split button opens the kind menu over the kit primitive', async () => {
  at('?state=all');
  const chevron = document.querySelector<HTMLButtonElement>(
    '.split button[aria-label="What to create"]',
  );
  assert.ok(chevron);
  testingLibrary.fireEvent.click(chevron);

  const menu = await testingLibrary.waitFor(() => {
    const found = document.querySelector('#overlays [role="menu"]');
    assert.ok(found, 'the menu is portaled out of the toolbar');
    return found;
  });
  const items = [...menu.querySelectorAll('button')];
  // The label is the item's own text with its shortcut taken off.
  const label = (item: HTMLButtonElement): string =>
    (item.textContent ?? '')
      .replace(item.querySelector('kbd')?.textContent ?? '', '')
      .trim();
  assert.deepEqual(items.map(label), ['Password', 'Resource', 'File', 'Link']);
  for (const item of items) assert.equal(item.disabled, false);
  testingLibrary.fireEvent.click(items[0]);
  await testingLibrary.waitFor(() =>
    assert.equal(text('.sheet h2'), 'New password'),
  );
});

/* --------------------------------------------------------------- phase 6 -- */

test('Servers is a live status list rather than the earlier placeholder', async () => {
  at('?state=servers');
  await flushPhase6Loads();
  assert.equal(text('.loc h1'), 'Servers & devices');
  assert.equal(text('.header-action'), 'Add a server…');
  assert.equal(document.querySelector('.main > .toolbar'), null);
  await testingLibrary.waitFor(() =>
    assert.equal(document.querySelectorAll('.server-card').length, FIXTURE.servers.length),
  );
  assert.equal(document.querySelector('.plain'), null);
  assert.doesNotMatch(
    text('.server-wrap'),
    /No account here yet|Account and group facts are not listed|Servers store encrypted data/,
  );
});

test('server details put status before Back and omit explanatory header copy', async () => {
  at('?state=servers-lapsed');
  await flushPhase6Loads();
  const controls = [
    ...document.querySelectorAll<HTMLElement>('.main > .toolbar > *'),
  ].map((node) => node.textContent?.trim() ?? '');
  assert.deepEqual(controls.slice(0, 2), [
    'Server check-in lapsed',
    '‹ Servers',
  ]);
  assert.doesNotMatch(
    text('.main > .path'),
    /account facts not listed|no account here yet/i,
  );
  assert.doesNotMatch(text('.main > .toolbar'), /Host facts from the latest check/);
});

const PHASE6_STATES = [
  ['servers-list', 'Servers & devices', null],
  ['servers-server', 'foks.example.net', null],
  ['servers-lapsed', 'foks.acme-corp.com', null],
  ['servers-rollback', 'foks.example.net', null],
  ['servers-reset', 'foks.example.net', 'Reset foks.example.net?'],
  ['servers-add', 'Servers & devices', 'Add a server'],
  ['servers-unprobed', 'foks.partner.dev', null],
  ['servers-check', 'foks.partner.dev', null],
  ['settings-macs', 'Settings', null],
  ['settings-phrase', 'Settings', 'Write these 17 tokens down'],
  ['settings-keys', 'Settings', null],
  ['settings-enrol', 'Settings', 'Create a YubiKey account'],
  ['settings-account', 'Settings', null],
  ['settings-agent', 'Settings', null],
  ['settings-about', 'Settings', null],
] as const;

for (const [stateName, title, sheetTitle] of PHASE6_STATES) {
  test(`Phase 6 state ${stateName} renders in the shared shell`, async () => {
    at(`?state=${stateName}`);
    await flushPhase6Loads();
    assert.equal(text('.loc h1'), title);
    if (sheetTitle) {
      await testingLibrary.waitFor(() => assert.equal(text('.sheet h2'), sheetTitle));
    } else {
      assert.equal(document.querySelector('.sheet'), null);
    }
    assert.equal(document.querySelector('.plain'), null);
  });
}

test('Settings account switch selects work without colliding with first-run account', async () => {
  at('?state=settings-macs-work');
  await testingLibrary.waitFor(() =>
    assert.match(text('.settings-main'), /MacBook Pro/),
  );
  // The exact store the address names, not the first account in the catalog.
  assert.equal(new URLSearchParams(window.location.search).get('store'), 'acct:work');
  assert.match(text('.settings-main'), /rae\.chen/);
  assert.doesNotMatch(text('.settings-main'), /Account access is stopped/);
  assert.match(text('.settings-main'), /MacBook Pro/);
  assert.equal(text('.loc h1'), 'Settings');
  assert.equal(document.querySelector('.main > .toolbar'), null);
});

test('Settings renders exact app version and socket from app_info', async () => {
  at('?state=settings-about');
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /FOKS Desktop 0\.3\.0/));
  testingLibrary.cleanup();
  at('?state=settings-agent');
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /\/private\/foks\/agent\.sock/));
  assert.equal([...document.querySelectorAll<HTMLButtonElement>('.settings-main button')].some((button) => /restart/i.test(button.textContent ?? '')), false);
});

test('server Check uses the explicit command and reports typed acceptance', async () => {
  const base = mockBridge(FIXTURE);
  let checks = 0;
  const bridge = { ...base, checkServer: async (profile: string) => {
    checks += 1;
    return base.checkServer(profile);
  } };
  at('?state=servers-unprobed', { bridge });
  await testingLibrary.waitFor(() => assert.ok(document.querySelector('.server-details')));
  const button = [...document.querySelectorAll<HTMLButtonElement>('button')].find((item) => item.textContent?.includes('Check now'));
  assert.ok(button);
  testingLibrary.fireEvent.click(button);
  await testingLibrary.waitFor(() => assert.equal(checks, 1));
  await testingLibrary.waitFor(() => assert.match(document.body.textContent ?? '', /New pin/));
});

test('the checked fixture scene re-reads its fresh signed status before enabling access', async () => {
  at('?state=servers-check');
  await testingLibrary.waitFor(() => assert.match(document.body.textContent ?? '', /New pin/));
  await testingLibrary.waitFor(() => assert.match(document.body.textContent ?? '', /Signed expiry/));
  assert.doesNotMatch(text('.server-wrap'), /reads and writes stopped/i);
});

test('a successful Check verdict remains visible when its passive status refresh fails', async () => {
  const base = mockBridge(FIXTURE); let partnerDescriptions = 0;
  const bridge = {
    ...base,
    native: true as const,
    describeServerStatus: async (profile: string) => {
      if (profile === 'partner' && ++partnerDescriptions > 1) throw new Error('passive status unavailable after check');
      return base.describeServerStatus(profile);
    },
  };
  at('?state=servers-unprobed', { bridge });
  await testingLibrary.waitFor(() => assert.ok(document.querySelector('.server-details')));
  const button = [...document.querySelectorAll<HTMLButtonElement>('button')].find((item) => item.textContent?.includes('Check now'));
  assert.ok(button); testingLibrary.fireEvent.click(button);
  await testingLibrary.waitFor(() => assert.match(document.body.textContent ?? '', /New pin/));
  await testingLibrary.waitFor(() => assert.match(document.body.textContent ?? '', /passive status unavailable after check/));
  assert.match(document.body.textContent ?? '', /New pin/);
});

test('reset previews exact resumables and consumes its authorization once', async () => {
  const base = mockBridge(FIXTURE);
  let calls = 0;
  const bridge = { ...base, resetServer: async (profile: string, confirmation: string, token: string) => {
    calls += 1;
    return base.resetServer(profile, confirmation, token);
  } };
  at('?state=servers-reset', { bridge });
  await flushPhase6Loads();
  await testingLibrary.waitFor(() => assert.match(text('.sheet'), /team-creation.*homelab/s));
  const input = document.querySelector<HTMLInputElement>('.sheet input');
  assert.ok(input);
  testingLibrary.fireEvent.change(input, { target: { value: 'personal' } });
  const resetButton = [...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find((item) => item.textContent?.includes('Reset local state'));
  assert.ok(resetButton);
  testingLibrary.fireEvent.click(resetButton);
  await testingLibrary.waitFor(() => assert.equal(calls, 1));
  await testingLibrary.waitFor(() => assert.match(text('.flash'), /Reset foks\.example\.net/));
  await flushPhase6Loads();
});

test('a failed reset cannot re-arm its consumed preview token on rerender', async () => {
  const base = mockBridge(FIXTURE);
  let calls = 0;
  const bridge = { ...base,
    describeServerStatus: () => new Promise<never>(() => undefined),
    resetServer: async () => {
    calls += 1;
    throw new Error('network stopped after submission');
  } };
  at('?state=servers-reset', { bridge });
  await testingLibrary.waitFor(() => assert.match(text('.sheet'), /team-creation/));
  const input = document.querySelector<HTMLInputElement>('.sheet input');
  assert.ok(input);
  testingLibrary.fireEvent.change(input, { target: { value: 'personal' } });
  const resetButton = [...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find((item) => item.textContent?.includes('Reset local state'));
  assert.ok(resetButton);
  testingLibrary.fireEvent.click(resetButton);
  await testingLibrary.waitFor(() => assert.equal(calls, 1));
  await testingLibrary.waitFor(() => assert.equal(resetButton.disabled, true));
  await testingLibrary.waitFor(() => assert.match(document.body.textContent ?? '', /network stopped after submission/));
  testingLibrary.fireEvent.click(resetButton);
  assert.equal(calls, 1);
  assert.match(text('.sheet'), /closing and reopening Reset/);
});

test('Add server profile validation matches the Rust bounded local name', () => {
  at('?state=servers-add');
  const inputs = document.querySelectorAll<HTMLInputElement>('.sheet input');
  const submit = [...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find((item) => item.textContent === 'Add server');
  assert.ok(submit);
  testingLibrary.fireEvent.change(inputs[0], { target: { value: 'Work_2' } });
  assert.equal(submit.disabled, false);
  testingLibrary.fireEvent.change(inputs[0], { target: { value: 'work.profile' } });
  assert.equal(submit.disabled, true);
  testingLibrary.fireEvent.change(inputs[0], { target: { value: 'x'.repeat(65) } });
  assert.equal(submit.disabled, true);
  testingLibrary.fireEvent.change(inputs[0], { target: { value: 'work' } });
  testingLibrary.fireEvent.change(inputs[1], { target: { value: 'a'.repeat(2049) } });
  assert.equal(submit.disabled, true);
  testingLibrary.fireEvent.change(inputs[1], { target: { value: 'é'.repeat(1025) } });
  assert.equal(submit.disabled, true);
});

test('pairing Start, Resume, Finish and Accept remain explicit and clear phrases', async () => {
  at('?state=settings-macs');
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Set up another Mac/));
  const start = [...document.querySelectorAll<HTMLButtonElement>('button')].find((item) => item.textContent?.includes('Start pairing'));
  assert.ok(start);
  testingLibrary.fireEvent.click(start);
  await testingLibrary.waitFor(() => assert.equal(text('.sheet h2'), 'Set up another Mac'));
  const startInside = [...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find((item) => item.textContent === 'Start');
  assert.ok(startInside);
  testingLibrary.fireEvent.click(startInside);
  await testingLibrary.waitFor(() => assert.match(text('.sheet'), /cobalt window/));
  assert.match(text('.sheet'), /Resume offer/);
  assert.match(text('.sheet'), /Finish/);
  const acceptTab = [...document.querySelectorAll<HTMLButtonElement>('.sheet .seg button')].find((item) => item.textContent === 'On this Mac');
  assert.ok(acceptTab);
  testingLibrary.fireEvent.click(acceptTab);
  assert.match(text('.sheet'), /Accept.*Resume acceptance/s);
  assert.equal(document.querySelector<HTMLInputElement>('input[aria-label="Pairing phrase"]')?.type ?? document.querySelectorAll<HTMLInputElement>('.sheet input')[2]?.type, 'password');
});

test('Accept or resume opens pairing on the receiving path', async () => {
  at('?state=settings-macs');
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Set up another Mac/));
  const acceptRoute = [...document.querySelectorAll<HTMLButtonElement>('.settings-main button')].find((item) => item.textContent?.includes('Accept or resume'));
  assert.ok(acceptRoute); testingLibrary.fireEvent.click(acceptRoute);
  await testingLibrary.waitFor(() => assert.equal(text('.sheet h2'), 'Set up another Mac'));
  assert.equal([...document.querySelectorAll<HTMLButtonElement>('.sheet .seg button')].find((item) => item.classList.contains('on'))?.textContent, 'On this Mac');
  assert.ok([...document.querySelectorAll<HTMLButtonElement>('.sheet button')].some((item) => item.textContent === 'Accept'));
});

test('a failed offer refresh clears the previously shown pairing phrase', async () => {
  const base = mockBridge(FIXTURE);
  const bridge = { ...base, resumeDevicePairingOffer: async () => { throw new Error('offer unavailable'); } };
  at('?state=settings-macs', { bridge });
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Set up another Mac/));
  testingLibrary.fireEvent.click([...document.querySelectorAll<HTMLButtonElement>('.settings-main button')].find((item) => item.textContent?.includes('Start pairing'))!);
  testingLibrary.fireEvent.click([...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find((item) => item.textContent === 'Start')!);
  await testingLibrary.waitFor(() => assert.match(text('.sheet'), /cobalt window/));
  testingLibrary.fireEvent.click([...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find((item) => item.textContent === 'Resume offer')!);
  await testingLibrary.waitFor(() => assert.doesNotMatch(text('.sheet'), /cobalt window/));
});

test('failed pairing acceptance clears its secret immediately and persists nothing', async () => {
  window.localStorage.clear();
  const base = mockBridge(FIXTURE);
  const bridge = { ...base, acceptDevicePairing: async () => { throw new Error('refused'); } };
  at('?state=settings-macs', { bridge });
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Set up another Mac/));
  const start = [...document.querySelectorAll<HTMLButtonElement>('button')].find((item) => item.textContent?.includes('Start pairing'));
  assert.ok(start); testingLibrary.fireEvent.click(start);
  const acceptTab = [...document.querySelectorAll<HTMLButtonElement>('.sheet .seg button')].find((item) => item.textContent === 'On this Mac');
  assert.ok(acceptTab); testingLibrary.fireEvent.click(acceptTab);
  const fields = document.querySelectorAll<HTMLInputElement>('.sheet input');
  testingLibrary.fireEvent.change(fields[2], { target: { value: 'secret offer phrase' } });
  const accept = [...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find((item) => item.textContent === 'Accept');
  assert.ok(accept); testingLibrary.fireEvent.click(accept);
  await testingLibrary.waitFor(() => assert.equal(fields[2].value, ''));
  assert.doesNotMatch(JSON.stringify(window.localStorage), /secret offer phrase/);
});

test('window blur closes a Settings secret sheet and drops its pairing offer', async () => {
  at('?state=settings-macs');
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Set up another Mac/));
  testingLibrary.fireEvent.click([...document.querySelectorAll<HTMLButtonElement>('.settings-main button')].find((button) => button.textContent === 'Start pairing…')!);
  testingLibrary.fireEvent.click([...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find((button) => button.textContent === 'Start')!);
  await testingLibrary.waitFor(() => assert.match(text('.sheet'), /cobalt window/));
  await testingLibrary.act(async () => { window.dispatchEvent(new Event('blur')); });
  assert.equal(document.querySelector('.sheet'), null);
  assert.doesNotMatch(document.body.textContent ?? '', /cobalt window/);
});

test('device removal is software-only and remains behind a danger confirmation sheet', async () => {
  const base = mockBridge(FIXTURE); let removed = 0;
  const bridge = {
    ...base,
    listAccountDevices: async () => [
      { id: `04${'4'.repeat(64)}`, role: 'owner' as const, current: false },
      { id: `08${'8'.repeat(66)}`, role: 'owner' as const, current: false },
    ],
    removeAccountDevice: async (_store: string, id: string) => { removed += 1; return { deviceId: id, userChainSequence: 2, alreadyAbsent: false }; },
  };
  at('?state=settings-macs', { bridge });
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /managed under Security keys/));
  assert.match(text('.settings-main'), /Device/);
  const removes = [...document.querySelectorAll<HTMLButtonElement>('.settings-main button')].filter((item) => item.textContent?.includes('Remove'));
  assert.equal(removes.length, 1);
  testingLibrary.fireEvent.click(removes[0]);
  assert.equal(removed, 0);
  assert.match(text('.sheet'), /Copies of values it already read cannot be recalled/);
  assert.equal([...document.querySelectorAll<HTMLButtonElement>('.sheet button')].filter((item) => item.classList.contains('danger')).length, 1);
  const confirm = document.querySelector<HTMLInputElement>('.sheet input');
  const remove = [...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find((item) => item.textContent === 'Remove device');
  assert.ok(confirm && remove);
  testingLibrary.fireEvent.change(confirm, { target: { value: 'wrong' } });
  assert.equal(remove.disabled, true);
  testingLibrary.fireEvent.change(confirm, { target: { value: `04${'4'.repeat(64)}` } });
  assert.equal(remove.disabled, false);
  testingLibrary.fireEvent.click(remove);
  await testingLibrary.waitFor(() => assert.equal(removed, 1));
});

test('a current Yubi device is not called this Mac', async () => {
  const base = mockBridge(FIXTURE);
  const bridge = { ...base, listAccountDevices: async () => [
    { id: `08${'8'.repeat(66)}`, role: 'owner' as const, current: true },
  ] };
  at('?state=settings-macs', { bridge });
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /current security key/));
  const deviceRow = [...document.querySelectorAll<HTMLElement>('.settings-inset .fr')].find((row) => row.textContent?.includes(`08${'8'.repeat(10)}`));
  assert.ok(deviceRow);
  assert.doesNotMatch(deviceRow.textContent ?? '', /this Mac/);
});

test('an expired passive lease stops Settings reads and every account action', async () => {
  const base = mockBridge(FIXTURE); let detailReads = 0;
  const bridge = {
    ...base,
    native: true as const,
    describeServerStatus: async (profile: string) => {
      const status = await base.describeServerStatus(profile);
      return profile === 'personal' ? { ...status, leaseExpiresAt: 1 } : status;
    },
    listAccountDevices: async (store: string) => { detailReads += 1; return base.listAccountDevices(store); },
    listBackupEnrollments: async (store: string) => { detailReads += 1; return base.listBackupEnrollments(store); },
    listYubiCards: async (profile: string) => { detailReads += 1; return base.listYubiCards(profile); },
    listYubiAccounts: async (profile: string) => { detailReads += 1; return base.listYubiAccounts(profile); },
  };
  at('?state=settings-macs', { bridge });
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Account access is stopped/));
  assert.equal(detailReads, 0);
  const actionButtons = [...document.querySelectorAll<HTMLButtonElement>('.settings-main button')].filter((button) => !button.closest('.settings-label'));
  assert.ok(actionButtons.length > 0);
  assert.equal(actionButtons.every((button) => button.disabled), true);
  await flushPhase6Loads();
});

test('Account passphrase buttons open the operation that was selected', async () => {
  at('?state=settings-account');
  await flushPhase6Loads();
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Device.*MacBook Pro.*Device.*Not listed while access is stopped/s));
  const change = [...document.querySelectorAll<HTMLButtonElement>('.settings-main button')].find((item) => item.textContent === 'Change…' && !item.disabled);
  assert.ok(change); testingLibrary.fireEvent.click(change);
  assert.equal([...document.querySelectorAll<HTMLButtonElement>('.sheet .seg button')].find((item) => item.classList.contains('on'))?.textContent, 'Change');
  assert.equal([...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button')].find((item) => item.classList.contains('primary'))?.textContent, 'Change passphrase');
});

test('each Account passphrase row targets its own opaque store', async () => {
  const base = mockBridge(FIXTURE); let target = '';
  const bridge = {
    ...base,
    describeServerStatus: async (profile: string) => {
      const status = await base.describeServerStatus(profile);
      return { ...status, leaseExpiresAt: Math.floor(Date.now() / 1000) + 86_400 };
    },
    changeAccountPassphrase: async (store: string) => {
      target = store;
      return { generation: 2, stretchVersion: 'v1' as const, verified: true as const };
    },
  };
  at('?state=settings&section=account&store=acct%3Awork', { bridge, world: applyLease(FIXTURE, 'fresh') });
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /signed check-in available/));
  const workBlock = [...document.querySelectorAll<HTMLElement>('.settings-main > div')].find((node) => node.textContent?.includes('Acme'));
  const change = [...(workBlock?.querySelectorAll<HTMLButtonElement>('button') ?? [])].find((button) => button.textContent === 'Change…');
  assert.ok(change); testingLibrary.fireEvent.click(change);
  const passwords = document.querySelectorAll<HTMLInputElement>('.sheet input[type="password"]');
  testingLibrary.fireEvent.change(passwords[0], { target: { value: 'new phrase' } });
  testingLibrary.fireEvent.change(passwords[1], { target: { value: 'new phrase' } });
  testingLibrary.fireEvent.click([...document.querySelectorAll<HTMLButtonElement>('.sheet button')].find((button) => button.textContent === 'Change passphrase')!);
  await testingLibrary.waitFor(() => assert.equal(target, 'acct:work'));
});

/* ------------------------------------- two profiles, one account alias -- */

/**
 * The collision an alias cannot express: two profiles, each holding an account
 * this Mac calls `personal`. These are the exact StoreRefs
 * `foks-tauri/src/commands.rs` writes, JSON punctuation and all.
 */
const SAME_ALIAS_A = '{"kind":"account","profile":"home","accountAlias":"personal"}';
const SAME_ALIAS_B = '{"kind":"account","profile":"acme-b","accountAlias":"personal"}';

/** A world holding both of them, in either catalog order, optionally minus one. */
function sharedAliasWorld({ order = 'a-first', without }: { order?: 'a-first' | 'b-first'; without?: string } = {}): World {
  const rows = [
    {
      store: {
        id: SAME_ALIAS_A, kind: 'account' as const, name: 'personal',
        server: 'home', account: 'personal',
      },
      account: { store: SAME_ALIAS_A, alias: 'personal', username: 'rae', server: 'home' },
    },
    {
      store: {
        id: SAME_ALIAS_B, kind: 'account' as const, name: 'personal',
        server: 'acme-b', account: 'personal',
      },
      account: { store: SAME_ALIAS_B, alias: 'personal', username: 'rae.chen', server: 'acme-b' },
    },
  ].filter((row) => row.store.id !== without);
  const ordered = order === 'b-first' ? [...rows].reverse() : rows;
  return {
    ...FIXTURE,
    servers: [
      { id: 'home', name: 'foks.home.example', label: 'Home', host_id: 'aa11223344', chain: 4, epoch: 91, lease: { state: 'fresh', expires_in: '6 d' }, accounts: ['personal'], state: 'ok' },
      { id: 'acme-b', name: 'foks.acme-corp.com', label: 'Acme', host_id: 'bb55667788', chain: 9, epoch: 42, lease: { state: 'fresh', expires_in: '6 d' }, accounts: ['personal'], state: 'ok' },
    ],
    accounts: ordered.map((row) => row.account),
    stores: ordered.map((row) => row.store),
    items: [],
    parties: [],
    federation: [],
    yubiAccounts: [],
  };
}

/** Records every StoreRef the account-specific commands are given. */
interface SharedAliasCalls {
  devices: string[];
  backups: string[];
  pairing: string[];
  removed: string[];
  passphrase: string[];
  phrase: [string, string][];
  recovery: [string, string][];
}

function sharedAliasBridge(world: World, calls: SharedAliasCalls, gate?: Map<string, (devices: AccountDevice[]) => void>): Bridge {
  const base = mockBridge(world);
  const device = (store: string): AccountDevice[] => [
    { id: `04${(store === SAME_ALIAS_A ? 'a' : 'b').repeat(64)}`, name: store === SAME_ALIAS_A ? 'Home Mac' : 'Acme Mac', role: 'owner', current: true },
  ];
  return {
    ...base,
    native: true,
    firstRunFixture: undefined,
    fixtureWorld: undefined,
    describeServerStatus: async (profile: string) => ({
      profile,
      configuredProbe: profile,
      host: { lookupName: profile, canonicalName: profile, hostId: `${profile}-host-id`, chain: 4, epoch: 91 },
      leaseRequired: true,
      leaseExpiresAt: Math.floor(Date.now() / 1000) + 86_400,
    }),
    listAccountDevices: async (store: string) => {
      calls.devices.push(store);
      if (gate) return new Promise<AccountDevice[]>((resolve) => gate.set(store, resolve));
      return device(store);
    },
    listBackupEnrollments: async (store: string) => { calls.backups.push(store); return []; },
    listYubiCards: async () => [],
    listYubiAccounts: async () => [],
    startDevicePairing: async (store: string) => { calls.pairing.push(store); return { accountAlias: 'personal', phrase: 'cobalt window ladder' }; },
    removeAccountDevice: async (store: string) => { calls.removed.push(store); return { deviceId: '04', userChainSequence: 2, alreadyAbsent: false }; },
    changeAccountPassphrase: async (store: string) => { calls.passphrase.push(store); return { generation: 2, stretchVersion: 'v1' as const, verified: true as const }; },
    prepareOwnerBackup: async (profile: string, alias: string, backupAlias: string) => { calls.phrase.push([profile, alias]); return { backupAlias, phrase: Array.from({ length: 17 }, (_, index) => `word${index}`).join(' ') }; },
    recoverOwnerAccount: async (profile: string, alias: string) => { calls.recovery.push([profile, alias]); return { applied: true }; },
  };
}

function noCalls(): SharedAliasCalls {
  return { devices: [], backups: [], pairing: [], removed: [], passphrase: [], phrase: [], recovery: [] };
}

/** The address bar as the shell has rewritten it. */
const storeParam = (): string | null =>
  new URLSearchParams(window.location.search).get('store');

const settingsAt = (store: string, section = 'macs'): string =>
  `?state=settings&section=${section}&store=${encodeURIComponent(store)}`;

test('Settings selects the account the address names, not the first one with that alias', async () => {
  const world = sharedAliasWorld();
  const calls = noCalls();
  at(settingsAt(SAME_ALIAS_B), { world, bridge: sharedAliasBridge(world, calls) });
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Acme Mac/));
  // A is listed first and shares the alias; B is the one selected and read.
  assert.equal(world.stores[0]?.id, SAME_ALIAS_A);
  assert.deepEqual(calls.devices, [SAME_ALIAS_B]);
  assert.deepEqual(calls.backups, [SAME_ALIAS_B]);
  assert.match(text('.settings-main'), /rae\.chen/);
  assert.doesNotMatch(text('.settings-main'), /Home Mac/);
  // Both switcher buttons carry their server, because the alias cannot tell
  // them apart on its own.
  assert.deepEqual(
    [...document.querySelectorAll<HTMLButtonElement>('.settings-label .seg button')].map((button) => button.textContent),
    ['personal · foks.home.example', 'personal · foks.acme-corp.com'],
  );
});

test('the selected account survives a catalog reordering', async () => {
  for (const order of ['a-first', 'b-first'] as const) {
    const world = sharedAliasWorld({ order });
    const calls = noCalls();
    at(settingsAt(SAME_ALIAS_B), { world, bridge: sharedAliasBridge(world, calls) });
    await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Acme Mac/));
    assert.deepEqual(calls.devices, [SAME_ALIAS_B], order);
    assert.equal(storeParam(), SAME_ALIAS_B, order);
    testingLibrary.cleanup();
  }
});

test('Settings carries the exact account across its sections', async () => {
  const world = sharedAliasWorld();
  const calls = noCalls();
  at(settingsAt(SAME_ALIAS_B), { world, bridge: sharedAliasBridge(world, calls) });
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Acme Mac/));
  for (const label of ['Account', 'Security keys', 'Your Macs & recovery']) {
    const nav = [...document.querySelectorAll<HTMLButtonElement>('.settings-sections button')]
      .find((button) => button.textContent?.trim() === label);
    assert.ok(nav, label);
    testingLibrary.fireEvent.click(nav);
    await testingLibrary.waitFor(() => assert.equal(storeParam(), SAME_ALIAS_B));
  }
  // Back on Macs, still reading B and only B. (The Account section reads every
  // account's current device on purpose — it lists them all — so `calls` is not
  // the assertion here; what this screen is *about* is.)
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Acme Mac/));
  assert.doesNotMatch(text('.settings-main'), /Home Mac/);
  assert.equal(storeParam(), SAME_ALIAS_B);
});

test('every account-specific Settings action carries the selected StoreRef', async () => {
  const world = sharedAliasWorld();
  const calls = noCalls();
  at(settingsAt(SAME_ALIAS_B), { world, bridge: sharedAliasBridge(world, calls) });
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Acme Mac/));

  const click = (label: string): void => {
    const button = [...document.querySelectorAll<HTMLButtonElement>('.settings-main button')]
      .find((candidate) => candidate.textContent?.trim() === label);
    assert.ok(button, label);
    testingLibrary.fireEvent.click(button);
  };
  const inSheet = (label: string): HTMLButtonElement => {
    const button = [...document.querySelectorAll<HTMLButtonElement>('.sheet button')]
      .find((candidate) => candidate.textContent?.trim() === label);
    assert.ok(button, label);
    return button;
  };

  click('Start pairing…');
  testingLibrary.fireEvent.click(inSheet('Start'));
  await testingLibrary.waitFor(() => assert.deepEqual(calls.pairing, [SAME_ALIAS_B]));
  testingLibrary.fireEvent.click(inSheet('Close'));

  click('Enrol…');
  testingLibrary.fireEvent.click(inSheet('Prepare phrase'));
  // The native call is (profile, alias) — an alias scoped by its profile, not
  // an alias on its own — and the profile is the selected account's.
  await testingLibrary.waitFor(() => assert.deepEqual(calls.phrase, [['acme-b', 'personal']]));
});

test('a Settings address naming an account that has gone says so instead of picking another', async () => {
  const world = sharedAliasWorld({ without: SAME_ALIAS_B });
  const calls = noCalls();
  at(settingsAt(SAME_ALIAS_B), { world, bridge: sharedAliasBridge(world, calls) });
  await testingLibrary.waitFor(() =>
    assert.match(text('.settings-main'), /no longer available in the current catalog/),
  );
  // A is the only account left and is emphatically not selected in B's place.
  assert.deepEqual(calls.devices, []);
  assert.doesNotMatch(text('.settings-main'), /Home Mac/);
  assert.equal(storeParam(), SAME_ALIAS_B);
});

test('choosing the other account is a deliberate route change', async () => {
  const world = sharedAliasWorld();
  const calls = noCalls();
  at(settingsAt(SAME_ALIAS_B), { world, bridge: sharedAliasBridge(world, calls) });
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Acme Mac/));
  const other = [...document.querySelectorAll<HTMLButtonElement>('.settings-label .seg button')]
    .find((button) => button.textContent?.includes('foks.home.example'));
  assert.ok(other);
  testingLibrary.fireEvent.click(other);
  await testingLibrary.waitFor(() => assert.equal(storeParam(), SAME_ALIAS_A));
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /Home Mac/));
  assert.deepEqual(calls.devices, [SAME_ALIAS_B, SAME_ALIAS_A]);
});

test('a late device answer for one account never lands under another', async () => {
  const world = sharedAliasWorld();
  const calls = noCalls();
  const gate = new Map<string, (devices: AccountDevice[]) => void>();
  at(settingsAt(SAME_ALIAS_A), { world, bridge: sharedAliasBridge(world, calls, gate) });
  await testingLibrary.waitFor(() => assert.deepEqual(calls.devices, [SAME_ALIAS_A]));

  const other = [...document.querySelectorAll<HTMLButtonElement>('.settings-label .seg button')]
    .find((button) => button.textContent?.includes('foks.acme-corp.com'));
  assert.ok(other);
  testingLibrary.fireEvent.click(other);
  await testingLibrary.waitFor(() => assert.deepEqual(calls.devices, [SAME_ALIAS_A, SAME_ALIAS_B]));

  // B answers, then A's earlier request answers late.
  await testingLibrary.act(async () => {
    gate.get(SAME_ALIAS_B)?.([{ id: `04${'b'.repeat(64)}`, name: 'Acme Mac', role: 'owner', current: true }]);
    await Promise.resolve();
    gate.get(SAME_ALIAS_A)?.([{ id: `04${'a'.repeat(64)}`, name: 'Home Mac', role: 'owner', current: true }]);
    for (let pass = 0; pass < 5; pass += 1) await new Promise((resolve) => setTimeout(resolve, 0));
  });
  assert.equal(storeParam(), SAME_ALIAS_B);
  assert.match(text('.settings-main'), /Acme Mac/);
  assert.doesNotMatch(text('.settings-main'), /Home Mac/);
});

/* ---------------------------------------- Join, with the same collision -- */

test('Join invites from the exact account, not the first with the alias', async () => {
  const world = sharedAliasWorld();
  const calls = noCalls();
  const copied: string[] = [];
  const base = sharedAliasBridge(world, calls);
  at('?state=join', { world, bridge: { ...base, copyText: async (text: string) => { copied.push(text); return { ok: true as const }; } } });
  const invites = [...document.querySelectorAll<HTMLButtonElement>('.plain button')]
    .filter((button) => button.textContent?.includes('Invite someone'));
  assert.equal(invites.length, 2);
  // The second card is profile B; its own box names B's server.
  const bInvite = invites.find((button) => button.closest('.copybox')?.textContent?.includes('rae.chen'));
  assert.ok(bInvite);
  assert.match(bInvite.closest('.copybox')?.textContent ?? '', /foks\.acme-corp\.com/);
  testingLibrary.fireEvent.click(bInvite);
  await testingLibrary.waitFor(() => assert.match(text('.sheet h2'), /foks\.acme-corp\.com/));
  assert.match(text('.sheet'), /rae\.chen/);
  assert.doesNotMatch(text('.sheet'), /foks\.home\.example/);
  testingLibrary.fireEvent.click([...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button')].find((button) => button.textContent === 'Copy message')!);
  await testingLibrary.waitFor(() => assert.equal(copied.length, 1));
  assert.match(copied[0], /foks\.acme-corp\.com/);
  assert.match(copied[0], /rae\.chen/);
  assert.doesNotMatch(copied[0], /foks\.home\.example/);
});

test('a Join invitation whose account leaves the catalog reports it rather than swapping', async () => {
  const world = sharedAliasWorld();
  const calls = noCalls();
  at('?state=join', { world, bridge: sharedAliasBridge(world, calls) });
  const bInvite = [...document.querySelectorAll<HTMLButtonElement>('.plain button')]
    .filter((button) => button.textContent?.includes('Invite someone'))
    .find((button) => button.closest('.copybox')?.textContent?.includes('rae.chen'));
  assert.ok(bInvite);
  testingLibrary.fireEvent.click(bInvite);
  await testingLibrary.waitFor(() => assert.match(text('.sheet h2'), /foks\.acme-corp\.com/));
  testingLibrary.cleanup();

  // The same open invitation, in a world that no longer holds that account.
  const shrunk = sharedAliasWorld({ without: SAME_ALIAS_B });
  at('?state=join-invite', { world: shrunk, bridge: sharedAliasBridge(shrunk, noCalls()) });
  await testingLibrary.waitFor(() => assert.match(text('.sheet h2'), /Invite to foks\.home\.example/));
  // The one account left is invited on its own terms, never as a stand-in for
  // the missing one: nothing here names the account that is gone.
  assert.match(text('.sheet'), /rae\b/);
  assert.doesNotMatch(text('.sheet'), /rae\.chen/);
});

test('Yubi actions bind pending and complete aliases and require their secret fields', async () => {
  const base = mockBridge(FIXTURE);
  const bridge = { ...base, listYubiAccounts: async () => [
    { alias: 'unfinished', state: 'pending' as const },
    { alias: 'daily', state: 'complete' as const },
  ] };
  at('?state=settings-keys', { bridge });
  await testingLibrary.waitFor(() => assert.match(text('.settings-main'), /unfinished.*daily/s));
  const rows = [...document.querySelectorAll<HTMLElement>('.settings-inset .fr')];
  const resume = rows.find((row) => row.textContent?.includes('Resume enrolment'))?.querySelector<HTMLButtonElement>('button');
  assert.ok(resume); testingLibrary.fireEvent.click(resume);
  assert.match(text('.sheet'), /unfinished/);
  const submit = [...document.querySelectorAll<HTMLButtonElement>('.sheet .ft button')].find((item) => item.textContent === 'Continue');
  assert.ok(submit?.disabled);
  testingLibrary.fireEvent.change(document.querySelector<HTMLInputElement>('.sheet input[type="password"]')!, { target: { value: '123456' } });
  assert.equal(submit.disabled, false);
});

test('rollback leaves host facts legible but every control except Back and Reset inert', async () => {
  at('?state=servers-rollback');
  await testingLibrary.waitFor(() => assert.match(text('.main'), /history does not match/));
  // The blocked notice renders from the route; the host facts arrive from the
  // status load. Wait for those too, so the controls this test asserts are
  // inert are actually on the page rather than not yet rendered.
  await testingLibrary.waitFor(() =>
    assert.ok(document.querySelector('[aria-label="They published"]')),
  );
  const details = document.querySelector<HTMLDetailsElement>('.server-details');
  assert.ok(details); details.open = true;
  const enabled = [...document.querySelectorAll<HTMLButtonElement>('.main button:not([disabled])')].map((button) => button.textContent?.trim());
  assert.deepEqual(enabled.sort(), ['Reset…', '‹ Servers'].sort());
  assert.equal(document.querySelector<HTMLInputElement>('[aria-label="They published"]')?.disabled, true);
  assert.doesNotMatch(text('.main'), /Never checked|Not pinned|Nothing yet/);
});

test('an unavailable signed status makes no claim that the server was never checked or pinned', async () => {
  const base = mockBridge(FIXTURE);
  const world: World = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) => server.id === 'personal'
      ? { ...server, host_id: null, chain: null, epoch: null, state: 'lease-unavailable' as const }
      : server),
  };
  at('?state=servers-server', {
    world,
    bridge: {
      ...base,
      native: true as const,
      fixtureWorld: undefined,
      describeServerStatus: async (profile: string) => {
        if (profile === 'personal') throw new Error('signed status unavailable');
        return base.describeServerStatus(profile);
      },
    },
  });
  await testingLibrary.waitFor(() => assert.match(text('.main'), /Host, pin and prior-check facts are not available/));
  assert.match(text('.main'), /Pin fact unavailable/);
  assert.doesNotMatch(text('.main'), /Never checked|Not pinned|Nothing yet|Added, never checked/);
});

test('an extreme signed expiry is displayed without crashing the server screen', async () => {
  const base = mockBridge(FIXTURE);
  const bridge = {
    ...base,
    describeServerStatus: async (profile: string) => ({
      ...(await base.describeServerStatus(profile)),
      leaseExpiresAt: Number.MAX_SAFE_INTEGER,
    }),
  };
  at('?state=servers-server', { bridge });
  await testingLibrary.waitFor(() =>
    assert.match(document.body.textContent ?? '', /Unix time 9007199254740991 seconds/),
  );
});
