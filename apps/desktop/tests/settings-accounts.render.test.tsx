/**
 * The Account tab: one account's profile at a time, the account the rail
 * header names. A "Switch account" button opens the same menu the rail
 * header's avatar opens, and the account's facts are rows with their action
 * at the right — the same workflows Settings › Accounts used to hold, behind
 * the same panels.
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

  // One account is shown at a time now, so each fact and each workflow
  // appears once: the five facts' own actions, and the three quieter links
  // below them.
  for (const name of [
    'Change…',
    'Settings › Servers',
    'Devices ›',
    'Teams ›',
    'Bot accounts',
    'Manage via web',
    'Organization sign-in',
  ])
    assert.equal(
      rendered.getAllByRole('button', { name }).length,
      1,
      `expected one "${name}" button`,
    );
  // The row names the account by the label this Mac gave it. The StoreRef the
  // address resolves against is opaque and is not drawn anywhere on the page.
  const aliasRow = rendered.getByText('Shown as').parentElement;
  assert.ok(aliasRow?.textContent?.includes('personal'));
  assert.equal(rendered.queryByText('acct:personal'), null);
  // The Join a team row is gone; Teams already offers its own join flow, so
  // this page does not repeat it.
  assert.equal(rendered.queryByRole('button', { name: 'Join a team…' }), null);
  assert.equal(
    rendered.queryByText(/Changing a username is a signed operation/),
    null,
  );
  assert.equal(rendered.queryByText('Check-in not required'), null);
});

test('the profile’s Teams and Devices facts summarize the account and link to where they are managed', async () => {
  const chosen: Location[] = [];
  const { rendered } = await renderPeople(
    await fixture(),
    'acct:personal',
    (location) => chosen.push(location),
  );

  // Teams: the names this account belongs to, not a row per team — each
  // team's own state is Teams' to show.
  const teamsButton = rendered.getByRole('button', { name: 'Teams ›' });
  const teamsRow = teamsButton.closest('.fr');
  assert.ok(teamsRow);
  assert.match(teamsRow?.textContent ?? '', /Household/);
  await ui.act(async () => {
    ui.fireEvent.click(teamsButton);
  });
  assert.deepEqual(chosen.at(-1), { kind: 'teams', store: 'acct:personal' });

  // Devices: a count, not the list — Devices already lists every key.
  const devicesButton = rendered.getByRole('button', { name: 'Devices ›' });
  const devicesRow = devicesButton.closest('.fr');
  assert.ok(devicesRow);
  assert.ok(
    ui.within(devicesRow as HTMLElement).getByText('5 devices and keys'),
  );
  await ui.act(async () => {
    ui.fireEvent.click(devicesButton);
  });
  assert.deepEqual(chosen.at(-1), { kind: 'devices', store: 'acct:personal' });

  // The Keys section is gone from this page.
  assert.equal(rendered.queryByText('MacBook Pro · Computer'), null);
  assert.equal(rendered.queryByText('paper-backup · Paper key'), null);
});

test('a group whose roster could not be read still names the team on the Teams fact', async () => {
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

  // The Teams fact is a plain list of names; whether a team's own roster
  // could be read is Teams' own state to show, not repeated here.
  const teamsButton = rendered.getByRole('button', { name: 'Teams ›' });
  assert.match(teamsButton.closest('.fr')?.textContent ?? '', /Household/);
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

  // The page counts this account's keys for the Devices fact, and asking
  // which card is in the port drives the card reader; nothing on this page
  // reports one.
  assert.equal(probes, 0);
});

test('a failed key read says so on the Devices fact rather than reporting no keys', async () => {
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

  const devicesButton = rendered.getByRole('button', { name: 'Devices ›' });
  const devicesRow = devicesButton.closest('.fr') as HTMLElement;
  assert.ok(
    ui.within(devicesRow).getByText('Devices and keys could not be read.'),
  );
  assert.equal(
    ui.within(devicesRow).queryByText('5 devices and keys'),
    null,
    'a read that failed is not an account with no keys',
  );
  // The failure itself is the shell's to report; Devices is where the read
  // is retried, so the detail is not repeated here.
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
  const devicesButton = rendered.getByRole('button', { name: 'Devices ›' });
  assert.ok(
    ui
      .within(devicesButton.closest('.fr') as HTMLElement)
      .getByText('5 devices and keys'),
  );
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
  const devicesRow = (): HTMLElement =>
    rendered
      .getByRole('button', { name: 'Devices ›' })
      .closest('.fr') as HTMLElement;
  // Wait for acct:work's own read to settle before taking the baseline, so
  // the comparison below is not measuring its own loading state.
  await ui.waitFor(() => {
    assert.doesNotMatch(devicesRow().textContent ?? '', /Reading this account/);
  });
  const before = devicesRow().textContent;
  await ui.act(async () => {
    release?.();
    await held;
    await Promise.resolve();
  });

  // The first account's keys answered late, and the page is about another
  // account now, so the late answer does not change what is shown.
  assert.equal(devicesRow().textContent, before);
});

test('a stopped account displays an unavailable notice on the Devices field and disables the link', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/index.ts',
  )) as typeof import('../src/model');
  const { rendered } = await renderPeople(
    applyLease(await fixture(), 'lapsed'),
    'acct:work',
  );

  const devicesButton = rendered.getByRole('button', { name: 'Devices ›' });
  assert.ok(
    ui
      .within(devicesButton.closest('.fr') as HTMLElement)
      .getByText('Not listed while access is stopped'),
  );
  assert.equal(devicesButton.hasAttribute('disabled'), true);
});

test('the username row opens a sheet titled for the workflow, not the account', async () => {
  const { rendered } = await renderPeople(await fixture());

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Change…' }));
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  const heading = ui.within(dialog).getByRole('heading', { level: 2 });
  assert.equal(heading.textContent, 'Change username');
  assert.equal(
    ui.within(dialog).queryByText('satoshi on Personal server'),
    null,
  );
  assert.ok(ui.within(dialog).getByLabelText('Username'));
});

test('the switcher lists every account and switching navigates by StoreRef', async () => {
  const chosen: Location[] = [];
  const { rendered } = await renderPeople(
    await fixture(),
    'acct:personal',
    (location) => chosen.push(location),
  );

  const trigger = rendered.getByRole('button', { name: 'Switch account' });
  assert.ok(
    trigger.closest('.phead'),
    'the switcher sits beside the account header',
  );
  await ui.act(async () => ui.fireEvent.click(trigger));
  const menu = rendered.getByRole('menu', { name: 'Accounts on this device' });
  assert.equal(menu.querySelectorAll('.acct').length, 2);
  assert.ok(ui.within(menu).getByLabelText('Current account'));
  assert.ok(
    ui.within(menu).getByRole('menuitem', { name: /Add account or server/ }),
  );
  await ui.act(async () =>
    ui.fireEvent.click(menu.querySelectorAll('.acct')[1]),
  );
  assert.deepEqual(chosen.at(-1), { kind: 'people', store: 'acct:work' });
});

test('a notice this page can route elsewhere is not repeated here', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/index.ts',
  )) as typeof import('../src/model');
  const lapsed = applyLease(await fixture(), 'lapsed');
  const { rendered } = await renderPeople(lapsed, 'acct:personal');

  // The fixture's three notes each resolve to a place of their own — Acme's
  // server page, Homelab's own page, and Engineering's admissions list — so
  // none of them are drawn on this page; it would be a second copy of a
  // state the reader can already see where it is acted on.
  assert.equal(rendered.container.querySelector('.people-attention'), null);
  assert.equal(rendered.queryByText('Acme is locked'), null);
  assert.equal(rendered.queryByText('Homelab is inactive'), null);
  assert.equal(rendered.queryByText('Homelab cannot access Engineering'), null);
});

test('with nothing needing attention the band is hidden', async () => {
  const snapshot = await fixture();
  const { rendered } = await renderPeople({ ...snapshot, notifications: [] });

  assert.equal(rendered.container.querySelector('.people-attention'), null);
  assert.equal(
    rendered.queryByRole('region', { name: /^Needs attention/ }),
    null,
  );
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
  await ui.act(async () =>
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Switch account' }),
    ),
  );
  const menu = rendered.getByRole('menu', { name: 'Accounts on this device' });
  assert.ok(ui.within(menu).getByText('Verification failed'));
  await ui.act(async () => ui.fireEvent.keyDown(menu, { key: 'Escape' }));
  for (const name of ['Bot accounts', 'Manage via web', 'Organization sign-in'])
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
  assert.ok(rendered.getByRole('button', { name: 'Refresh' }));
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'personal · Personal server' }),
    );
  });
  assert.deepEqual(chosen.at(-1), { kind: 'people', store: 'acct:personal' });
});

test('organization sign-in survives browser focus and can finish the same flow', async () => {
  const { rendered } = await renderPeople(await fixture());
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Organization sign-in' }),
    );
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

test('local alias save uses the stable account selector and retains input on failure', async () => {
  const calls: [string, string][] = [];
  let fail = true;
  const { rendered, refreshed } = await renderPeople(
    await fixture(),
    'acct:personal',
    () => {},
    {
      decorate: (base) => ({
        ...base,
        setLocalAccountAlias: async (store, label) => {
          calls.push([store, label]);
          if (fail) throw new Error('Could not save local alias.');
          return { store, alias: label };
        },
      }),
    },
  );
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Change local alias' }),
  );
  const dialog = rendered.getByRole('dialog', { name: 'Change local alias' });
  const field = ui.within(dialog).getByRole('textbox', { name: 'Local alias' });
  ui.fireEvent.change(field, { target: { value: '  Private account  ' } });
  ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Save' }));
  await rendered.findByText('Could not save local alias.');
  assert.equal((field as HTMLInputElement).value, '  Private account  ');
  fail = false;
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Save' }));
  });
  await ui.waitFor(() => assert.ok(!rendered.queryByRole('dialog')));
  assert.deepEqual(calls, [
    ['acct:personal', 'Private account'],
    ['acct:personal', 'Private account'],
  ]);
  assert.deepEqual(refreshed, ['Local alias updated']);
});

test('local alias appears in account controls while commands keep the original alias', async () => {
  const original = await fixture();
  const snapshot = {
    ...original,
    accounts: original.accounts.map((a) =>
      a.store === 'acct:personal' ? { ...a, localAlias: 'Private account' } : a,
    ),
  };
  const calls: string[] = [];
  const { rendered } = await renderPeople(snapshot, 'acct:personal', () => {}, {
    decorate: (base) => ({
      ...base,
      renameAccount: async (profile, alias, action) => {
        calls.push(alias);
        return base.renameAccount(profile, alias, action);
      },
    }),
  });
  assert.ok(rendered.getAllByText('Private account').length >= 2);
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Switch account' }));
  assert.ok(ui.within(rendered.getByRole('menu')).getByText('Private account'));
  ui.fireEvent.keyDown(rendered.getByRole('menu'), { key: 'Escape' });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Change…' }));
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Show pending changes' }),
  );
  await ui.waitFor(() => assert.deepEqual(calls, ['personal']));
});
