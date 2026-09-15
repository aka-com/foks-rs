/**
 * The People tab, which is where Settings → Accounts went: the attention list
 * with an action on every card, the account switcher, and the account panel
 * whose rows open the same workflows the Accounts pane opened.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge } from '../src/bridge';
import type { Location } from '../src/location';
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

async function fixture(): Promise<AgentSnapshot> {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  return FIXTURE;
}

interface PeopleOptions {
  /** Wraps the mock bridge, for a test that fails or delays one call. */
  decorate?: (bridge: Bridge) => Bridge;
  /** Collects what the page reports instead of failing the test on it. */
  collectErrors?: boolean;
}

async function renderPeople(
  snapshot: AgentSnapshot,
  store: string | undefined = 'acct:personal',
  onNavigate: (location: Location) => void = () => {},
  { decorate = (bridge) => bridge, collectErrors = false }: PeopleOptions = {},
) {
  const { PeopleScreen } = (await vite.ssrLoadModule(
    '/src/screens/people-screen.tsx',
  )) as typeof import('../src/screens/people-screen');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { ToastController, ToastProvider } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const refreshed: string[] = [];
  const reported: unknown[] = [];
  const bridge = decorate(mockBridge(snapshot));
  const controller = new ToastController();
  const page = (at: string | undefined) =>
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller,
        children: createElement(PeopleScreen, {
          snapshot,
          bridge,
          location: { kind: 'people', ...(at ? { store: at } : {}) },
          onNavigate,
          onRefresh: async (message: string) => {
            refreshed.push(message);
          },
          onRefreshSnapshot: async () => {
            refreshed.push('snapshot');
            return snapshot;
          },
          onError: (error: unknown) => {
            if (!collectErrors) throw error;
            reported.push(error);
          },
          onMutationError: async (error: unknown) => {
            throw error;
          },
        }),
      }),
    });
  const rendered = ui.render(page(store));
  await ui.act(async () => {
    await Promise.resolve();
  });
  /** Re-address the page at another account, as the switcher does. */
  const showAccount = async (at: string): Promise<void> => {
    await ui.act(async () => {
      rendered.rerender(page(at));
      await Promise.resolve();
    });
  };
  return { rendered, refreshed, reported, showAccount };
}

test('the account panel keeps every workflow row from the accounts pane', async () => {
  const { rendered } = await renderPeople(await fixture());

  // One account is shown at a time now, so each workflow appears once.
  for (const name of [
    'Change…',
    'Sign in…',
    'Manage…',
    'Open…',
    'Join a group…',
    'Settings › Account',
  ])
    assert.equal(
      rendered.getAllByRole('button', { name }).length,
      1,
      `expected one "${name}" button`,
    );
  // The row names the account by the label this Mac gave it. The StoreRef the
  // address resolves against is opaque and is not drawn anywhere on the page.
  const aliasRow = rendered.getByText('Local alias').parentElement;
  assert.ok(aliasRow?.textContent?.includes('personal'));
  assert.equal(rendered.queryByText('acct:personal'), null);
  // The device facts are the two sections above these rows; a third row
  // repeating one of them was removed, and Devices is reached from the
  // section that lists them.
  assert.equal(rendered.queryByRole('button', { name: 'Open Devices' }), null);
  assert.equal(
    rendered.getAllByRole('button', { name: 'Manage devices' }).length,
    1,
  );
  // The row is named for the panel behind it, which is SsoPanel's own title.
  assert.ok(rendered.getByText('Organization sign-in'));
  // Changing a username is a server operation, and the row says so.
  assert.ok(
    rendered.getByText(
      'Changing a username is a signed operation on the server; the local alias does not change with it.',
    ),
  );
});

test('the profile lists the account’s teams, its devices and its keys', async () => {
  const chosen: Location[] = [];
  const { rendered } = await renderPeople(
    await fixture(),
    'acct:personal',
    (location) => chosen.push(location),
  );

  const teams = rendered.getByRole('region', { name: 'Teams you’re in' });
  assert.ok(ui.within(teams).getByText('Household'));
  // The role is bare, and a group that is not set up says so as a chip at the
  // end of its row.
  assert.ok(ui.within(teams).getByText('Owner'));
  assert.ok(ui.within(teams).getByText('Setup incomplete'));
  // The caption says what the object is, as it does on Teams and on a
  // server's page — without the server, which the band above already names.
  assert.ok(ui.within(teams).getByText(/^Named group · /));
  assert.equal(ui.within(teams).queryByText(/Named group · foks/), null);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(teams).getByRole('button', { name: 'Open Homelab in Teams' }),
    );
  });
  assert.deepEqual(chosen.at(-1), {
    kind: 'group-settings',
    ref: 'team:homelab',
    tab: 'people',
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(teams).getByRole('button', { name: 'All teams' }),
    );
  });
  assert.deepEqual(chosen.at(-1), { kind: 'teams', store: 'acct:personal' });

  // Two Macs, a key on a card, a paper key and an enrollment, counted and
  // named.
  const devices = rendered.getByRole('region', { name: 'Devices' });
  assert.ok(ui.within(devices).getByText('5 devices and keys'));
  assert.ok(ui.within(devices).getByText(/MacBook Pro · Travel Mac/));
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(devices).getByRole('button', { name: 'Manage devices' }),
    );
  });
  assert.deepEqual(chosen.at(-1), { kind: 'devices', store: 'acct:personal' });

  const keys = rendered.getByRole('region', { name: 'Keys' });
  assert.ok(ui.within(keys).getByText('MacBook Pro · Computer'));
  assert.ok(ui.within(keys).getByText('paper-backup · Paper key'));
  // The id leads the row; what it belongs to follows it.
  const rows = keys.querySelectorAll('.keyline');
  assert.match(
    rows[0].querySelector('b')?.textContent ?? '',
    /^04a779c406…/,
    'the shortened key id is the row’s first text',
  );
  // An enrollment is answered for the server profile, not for this account,
  // and its row says where it is enrolled rather than naming an account.
  assert.ok(ui.within(keys).getByText('Enrollment · on foks.example.net'));
  // The only verification this page can state is this Mac's own key.
  assert.equal(
    ui.within(keys).getAllByRole('img', { name: 'Authenticated on this Mac' })
      .length,
    1,
  );
});

test('a group whose roster could not be read draws the chip, not the caption', async () => {
  const snapshot = await fixture();
  const { rendered } = await renderPeople({
    ...snapshot,
    // A roster that could not be read leaves no parties behind, so the
    // fixture's are dropped with it.
    parties: snapshot.parties.filter(
      (party) => party.store !== 'team:household',
    ),
    groupDetailFailures: [
      {
        store: 'team:household',
        source: 'roster',
        code: 'unavailable',
        message: 'The group roster could not be read from the agent.',
        retryable: true,
      },
    ],
  });

  const teams = rendered.getByRole('region', { name: 'Teams you’re in' });
  const row = [...teams.querySelectorAll('.fr.devrow')].find((entry) =>
    entry.textContent?.includes('Household'),
  );
  assert.ok(row, 'the account still lists the group');
  // The state is a chip at the end of the row; the caption says only what the
  // object is, because the summary it would carry could not be read.
  assert.equal(
    [...row.querySelectorAll('.chip')].map((chip) => chip.textContent)[0],
    'Roster unavailable',
  );
  assert.equal(row.querySelector('small')?.textContent, 'Named group');
});

test('People lists no connected card, so it never drives the reader', async () => {
  let probes = 0;
  await renderPeople(await fixture(), 'acct:personal', () => {}, {
    decorate: (bridge) => ({
      ...bridge,
      listYubiCards: async (profile) => {
        probes += 1;
        return bridge.listYubiCards(profile);
      },
    }),
  });

  // The page lists this account's keys, and asking which card is in the port
  // drives the card reader; nothing on this page reports one.
  assert.equal(probes, 0);
});

test('a failed key read says so rather than reporting no keys', async () => {
  const failure = new Error('the agent did not answer');
  const { rendered, reported } = await renderPeople(
    await fixture(),
    'acct:personal',
    () => {},
    {
      collectErrors: true,
      decorate: (bridge) => ({
        ...bridge,
        listAccountDevices: async () => {
          throw failure;
        },
      }),
    },
  );

  const keys = rendered.getByRole('region', { name: 'Keys' });
  assert.ok(ui.within(keys).getByText('Keys could not be read.'));
  assert.ok(ui.within(keys).getByText('Unavailable'));
  assert.ok(
    ui
      .within(keys)
      .getByText(
        'Could not retrieve keys from the background service. Open Devices to retry or check service status.',
      ),
  );
  assert.equal(
    ui
      .within(keys)
      .queryByText('No keys are listed for this account on this Mac.'),
    null,
    'a read that failed is not an account with no keys',
  );
  const devices = rendered.getByRole('region', { name: 'Devices' });
  assert.ok(
    ui.within(devices).getByText('Devices and keys could not be read.'),
  );
  // The failure itself is the shell's to report.
  assert.deepEqual(reported, [failure]);
});

test('a catalog replaced mid-read is recovered, and the keys arrive', async () => {
  let attempts = 0;
  const { rendered, refreshed } = await renderPeople(
    await fixture(),
    'acct:personal',
    () => {},
    {
      decorate: (bridge) => ({
        ...bridge,
        listAccountDevices: async (store) => {
          attempts += 1;
          if (attempts === 1)
            throw {
              code: 'catalog-required',
              message: 'The vault changed while it was loading.',
              retryable: true,
              ambiguous: false,
              fatal: false,
            };
          return bridge.listAccountDevices(store);
        },
      }),
    },
  );

  // The shell's own refresh is what recovers it, and the read is tried again.
  assert.ok(refreshed.includes('snapshot'));
  assert.equal(attempts, 2);
  const keys = rendered.getByRole('region', { name: 'Keys' });
  assert.ok(ui.within(keys).getByText('MacBook Pro · Computer'));
});

test('a read answering after the account changed is dropped', async () => {
  let release: (() => void) | undefined;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  const { rendered, showAccount } = await renderPeople(
    await fixture(),
    'acct:personal',
    () => {},
    {
      decorate: (bridge) => ({
        ...bridge,
        listAccountDevices: async (store) => {
          if (store === 'acct:personal') await held;
          return bridge.listAccountDevices(store);
        },
      }),
    },
  );

  await showAccount('acct:work');
  await ui.act(async () => {
    release?.();
    await held;
    await Promise.resolve();
  });

  // The first account's keys answered late, and the page is about another
  // account now, so they are not listed under it.
  const keys = rendered.getByRole('region', { name: 'Keys' });
  assert.equal(ui.within(keys).queryByText('Travel Mac · Computer'), null);
  assert.equal(ui.within(keys).queryByText('paper-backup · Paper key'), null);
  assert.ok(ui.within(keys).getByText('Enrollment · on foks.acme-corp.com'));
});

test('a stopped account lists no keys and says what is unavailable', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/index.ts',
  )) as typeof import('../src/model');
  const { rendered } = await renderPeople(
    applyLease(await fixture(), 'lapsed'),
    'acct:work',
  );

  const keys = rendered.getByRole('region', { name: 'Keys' });
  assert.ok(
    ui
      .within(keys)
      .getByText('Keys are unavailable until foks.acme-corp.com is checked.'),
  );
  const devices = rendered.getByRole('region', { name: 'Devices' });
  assert.ok(ui.within(devices).getByText('Not listed while access is stopped'));
  assert.equal(
    ui
      .within(devices)
      .getByRole('button', { name: 'Manage devices' })
      .hasAttribute('disabled'),
    true,
  );
});

test('the username row opens a sheet titled for the workflow, not the account', async () => {
  const { rendered } = await renderPeople(await fixture());

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Change…' }));
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  const heading = ui.within(dialog).getByRole('heading', { level: 2 });
  assert.equal(heading.textContent, 'Change username');
  assert.ok(ui.within(dialog).getByText('satoshi on foks.example.net'));
  assert.ok(ui.within(dialog).getByLabelText('Username'));
});

test('the switcher lists every account and switching navigates by StoreRef', async () => {
  const chosen: Location[] = [];
  const { rendered } = await renderPeople(
    await fixture(),
    'acct:personal',
    (location) => chosen.push(location),
  );

  // The switcher is named by the label above it, the one the reader sees.
  const group = rendered.getByRole('group', { name: 'Accounts on this Mac' });
  const accounts = ui.within(group).getAllByRole('button');
  assert.equal(accounts.length, 2);
  assert.equal(accounts[0].getAttribute('aria-pressed'), 'true');
  assert.ok(accounts[0].textContent?.includes('satoshi'));
  assert.ok(accounts[1].textContent?.includes('foks.acme-corp.com'));

  await ui.act(async () => {
    ui.fireEvent.click(accounts[1]);
  });
  assert.deepEqual(chosen.at(-1), { kind: 'people', store: 'acct:work' });
});

test('an attention card carries the route to where it is resolved', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/index.ts',
  )) as typeof import('../src/model');
  const chosen: Location[] = [];
  const lapsed = applyLease(await fixture(), 'lapsed');
  const { rendered } = await renderPeople(lapsed, 'acct:personal', (location) =>
    chosen.push(location),
  );

  assert.ok(rendered.getByText('foks.acme-corp.com is locked'));
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Open the server' }),
    );
  });
  assert.deepEqual(chosen.at(-1), {
    kind: 'settings',
    section: 'servers',
    profile: 'acme',
  });
  // A `team-` note routes to the group it names.
  assert.equal(
    rendered.getAllByRole('button', { name: 'Open Homelab' }).length,
    1,
  );
  // A `fed-` note is Homelab's admission into Engineering, and Engineering is
  // where it is restored.
  assert.ok(rendered.getByRole('button', { name: 'Open Engineering' }));
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Open Engineering' }),
    );
  });
  assert.deepEqual(chosen.at(-1), {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'people',
  });
  // A card that leads somewhere says where, and keeps the agent's own word
  // for what it is asking.
  assert.ok(rendered.getByText('Required action: Restore access'));
  assert.ok(rendered.getByText('Required action: Check status'));
});

test('a team note with the same alias on two profiles routes to neither', async () => {
  const snapshot = await fixture();
  const ambiguous: AgentSnapshot = {
    ...snapshot,
    stores: snapshot.stores.map((store) =>
      store.id === 'team:household' ? { ...store, alias: 'homelab' } : store,
    ),
  };
  const { rendered } = await renderPeople(ambiguous);

  assert.equal(rendered.queryByRole('button', { name: 'Open Homelab' }), null);
  const list = rendered.getByRole('region', { name: /^Needs attention/ });
  assert.ok(ui.within(list).getByText('Resume creation'));
});

test('a fed- note admitted by two hosts routes to neither', async () => {
  const snapshot = await fixture();
  const ambiguous: AgentSnapshot = {
    ...snapshot,
    federation: [
      ...snapshot.federation,
      {
        ...snapshot.federation[0],
        store: 'team:household',
        operation_id_hex: '7c14a9f1',
      },
    ],
  };
  const { rendered } = await renderPeople(ambiguous);

  // Homelab is admitted into two groups on this Mac, so which one restores the
  // admission is a guess; the note says what the agent asks for instead.
  assert.equal(
    rendered.queryByRole('button', { name: 'Open Engineering' }),
    null,
  );
  assert.equal(
    rendered.queryByRole('button', { name: 'Open Household' }),
    null,
  );
  const list = rendered.getByRole('region', { name: /^Needs attention/ });
  assert.ok(ui.within(list).getByText('Restore access'));
});

test('an admission already restored does not route the note either', async () => {
  const snapshot = await fixture();
  const restored: AgentSnapshot = {
    ...snapshot,
    federation: snapshot.federation.map((entry) => ({
      ...entry,
      active: true,
    })),
  };
  const { rendered } = await renderPeople(restored);

  assert.equal(
    rendered.queryByRole('button', { name: 'Open Engineering' }),
    null,
  );
});

test('unlinked notifications display the agent action label as a chip', async () => {
  const snapshot = await fixture();
  const orphan: AgentSnapshot = {
    ...snapshot,
    notifications: [
      {
        id: 'fed-nowhere',
        severity: 'warn',
        title: 'An admission needs attention',
        detail: 'The catalog does not say which group holds it.',
        action: 'Restore access',
      },
    ],
  };
  const { rendered } = await renderPeople(orphan);

  // No wrong page is offered; the chip reads what the agent asked for, and
  // the caption that would repeat it word for word is not drawn.
  const list = rendered.getByRole('region', { name: /^Needs attention/ });
  assert.ok(ui.within(list).getByText('An admission needs attention'));
  assert.equal(ui.within(list).queryByRole('button'), null);
  assert.ok(ui.within(list).getByText('Restore access'));
  assert.equal(
    ui.within(list).queryByText(/the agent reports this as/),
    null,
    'the caption does not repeat the chip',
  );
});

test('the catalog note keeps the live retry, and it reloads the snapshot', async () => {
  const snapshot = await fixture();
  const withCatalog: AgentSnapshot = {
    ...snapshot,
    notifications: [
      {
        id: 'catalog-catalog-0',
        severity: 'warn',
        title: 'Could not load catalog on acme',
        detail: 'The catalog request to foks.acme-corp.com did not complete.',
        action: 'Retry',
      },
    ],
  };
  const { rendered, refreshed } = await renderPeople(withCatalog);

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Retry' }));
  });
  assert.ok(refreshed.includes('snapshot'));
});

test('a stopped account disables username changes while recovery remains enabled', async () => {
  const snapshot = await fixture();
  const blocked: AgentSnapshot = {
    ...snapshot,
    servers: snapshot.servers.map((server) =>
      server.id === 'personal'
        ? {
            ...server,
            trust: {
              status: 'blocked' as const,
              error: {
                code: 'verification-failed',
                message: 'Pinned server history changed.',
                retryable: false,
                ambiguous: false,
                fatal: true,
              },
            },
          }
        : server,
    ),
  };
  const { rendered } = await renderPeople(blocked);

  const rename = rendered.getByRole('button', { name: 'Change…' });
  assert.equal(rename.hasAttribute('disabled'), true);
  assert.match(rename.getAttribute('title') ?? '', /Account access is stopped/);
  assert.ok(rendered.getByText('Account access is stopped'));
  // The reason is on the switcher row itself, not only in its tooltip.
  const group = rendered.getByRole('group', { name: 'Accounts on this Mac' });
  assert.ok(
    ui.within(group).getByText(/^Account access is stopped · /),
    'the stopped account says why on its row',
  );
  for (const name of ['Sign in…', 'Manage…', 'Open…'])
    assert.equal(
      rendered.getByRole('button', { name }).hasAttribute('disabled'),
      false,
      `${name} is a way back in and stays available`,
    );
});

test('a stale address says the account is no longer available', async () => {
  const chosen: Location[] = [];
  const { rendered } = await renderPeople(
    await fixture(),
    'acct:removed',
    (location) => chosen.push(location),
  );

  assert.ok(rendered.getByText('Account no longer available'));
  assert.ok(rendered.getByRole('button', { name: 'Refresh the catalog' }));
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'personal · foks.example.net' }),
    );
  });
  assert.deepEqual(chosen.at(-1), { kind: 'people', store: 'acct:personal' });
});

test('organization sign-in survives browser focus and can finish the same flow', async () => {
  const { rendered } = await renderPeople(await fixture());
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Sign in…' }));
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Sign in' }));
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Open sign-in browser' }),
    );
    window.dispatchEvent(new window.Event('blur'));
  });
  assert.ok(rendered.getByRole('dialog'));
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Check sign-in' }));
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Finish sign-in' }),
    );
  });
  assert.ok(
    rendered.getByText('Account authentication and service access verified.'),
  );
});
