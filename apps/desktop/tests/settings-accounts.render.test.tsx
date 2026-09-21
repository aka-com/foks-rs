/**
 * Settings › Account: one account's profile at a time, the account the rail
 * header names. The header carries no switcher — the rail header's avatar
 * menu is the only one — and the account's facts are rows with their action
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
  /** Optional callback to pause during post-mutation refresh for testing pending UI states. */
  holdRefresh?: () => Promise<void>;
}

async function renderPeople(
  snapshot: AgentSnapshot,
  store: string | undefined = 'acct:personal',
  onNavigate: (location: Location) => void = () => {},
  {
    decorate = (bridge) => bridge,
    collectErrors = false,
    holdRefresh,
  }: PeopleOptions = {},
) {
  const { AccountSection } = (await vite.ssrLoadModule(
    '/src/screens/account-section.tsx',
  )) as typeof import('../src/screens/account-section');
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
  /** Recorded refresh calls formatted as `[message, profile]` tuples. */
  const readBacks: [string, string | undefined][] = [];
  const reported: unknown[] = [];
  const bridge = decorate(mockBridge(snapshot));
  const controller = new ToastController();
  const page = (at: string | undefined) =>
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller,
        children: createElement(AccountSection, {
          snapshot,
          bridge,
          location: {
            kind: 'settings',
            section: 'account',
            ...(at ? { store: at } : {}),
          },
          panel: { id: 'settings-sections-panel-account', labelledBy: 'tab' },
          onNavigate,
          onRefresh: async (message: string, profile?: string) => {
            refreshed.push(message);
            readBacks.push([message, profile]);
            await holdRefresh?.();
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
  return { rendered, refreshed, readBacks, reported, showAccount };
}

test('an ungranted device capability does not invent a missing backup key', async () => {
  const base = await fixture();
  const snapshot: AgentSnapshot = {
    ...base,
    servers: base.servers.map((server) =>
      server.id === 'personal'
        ? {
            ...server,
            compatibility: {
              status: 'required',
              expiresAt: 4e9,
              capabilities: ['kv'],
            },
          }
        : server,
    ),
  };
  let reads = 0;
  let observed!: Bridge;
  const { rendered } = await renderPeople(
    snapshot,
    'acct:personal',
    undefined,
    {
      decorate: (bridge) => {
        observed = {
          ...bridge,
          listAccountDevices: async () => {
            reads++;
            return [];
          },
        };
        return observed;
      },
    },
  );
  const { deviceAlertRegistry } = (await vite.ssrLoadModule(
    '/src/screens/device-alert.ts',
  )) as typeof import('../src/screens/device-alert');
  assert.equal(reads, 0);
  assert.equal(
    deviceAlertRegistry(observed).getSnapshot().paperKeys.has('acct:personal'),
    false,
  );
  assert.equal(
    (rendered.getByRole('button', { name: 'Devices ›' }) as HTMLButtonElement)
      .disabled,
    true,
  );
});

test('the account panel keeps every workflow row from the accounts pane', async () => {
  const { rendered } = await renderPeople(await fixture());

  // One account is shown at a time now, so each fact and each workflow
  // appears once: the primary facts' actions, the passphrase action, and the
  // six quieter links below the device-wide server inventory.
  for (const name of [
    'Change…',
    'Server details ›',
    'Devices ›',
    'Teams ›',
    'Passphrase…',
    'Bot accounts',
    'Web admin panel',
    'Sign in via SSO',
    'Import from FOKS CLI',
    'Connect an existing account with a paper key',
    'Create an account on a hardware key…',
  ])
    assert.equal(
      rendered.getAllByRole('button', { name }).length,
      1,
      `expected one "${name}" button`,
    );
  for (const name of ['Server details ›', 'Devices ›', 'Teams ›'])
    assert.ok(
      rendered
        .getByRole('button', { name })
        .classList.contains('account-fact-link'),
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
  assert.equal(
    rendered.queryByText(
      'Set or change this account’s passphrase on its server.',
    ),
    null,
  );
  assert.deepEqual(
    [...document.querySelectorAll('.account-more button')].map((button) =>
      button.textContent?.trim(),
    ),
    [
      'Bot accounts',
      'Web admin panel',
      'Sign in via SSO',
      'Import from FOKS CLI',
      'Connect an existing account with a paper key',
      'Create an account on a hardware key…',
    ],
  );
});

test('account recovery and hardware-key creation open after the existing actions', async () => {
  const { rendered } = await renderPeople(await fixture());

  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', {
        name: 'Connect an existing account with a paper key',
      }),
    );
  });
  let dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(
    ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
    'Connect account via recovery key',
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Cancel' }),
    );
  });

  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', {
        name: 'Create an account on a hardware key…',
      }),
    );
  });
  dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(
    ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
    'Create an account on a hardware key',
  );
});

test('the profile’s Teams and Devices facts summarize the account and link to where they are managed', async () => {
  const chosen: Location[] = [];
  const { rendered } = await renderPeople(
    await fixture(),
    'acct:personal',
    (location) => chosen.push(location),
  );

  // Teams: a count rather than a list — each team's own state is Teams' to show.
  const teamsButton = rendered.getByRole('button', { name: 'Teams ›' });
  const teamsRow = teamsButton.closest('.fr');
  assert.ok(teamsRow);
  assert.ok(ui.within(teamsRow as HTMLElement).getByText('2 teams'));
  assert.ok(teamsRow?.parentElement?.classList.contains('middle'));
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

test('a team whose roster could not be read remains in the Teams count', async () => {
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

  // The Teams fact remains a count when one team's roster cannot be read.
  const teamsButton = rendered.getByRole('button', { name: 'Teams ›' });
  assert.match(teamsButton.closest('.fr')?.textContent ?? '', /2 teams/);
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

test('losing focus keeps a masked account workflow open', async () => {
  const { rendered } = await renderPeople(await fixture());
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Change…' }));
  const dialog = await rendered.findByRole('dialog');

  ui.fireEvent(window, new Event('blur'));

  assert.equal(rendered.getByRole('dialog'), dialog);
  assert.equal(
    rendered
      .getByLabelText('Security key PIN (enrolled keys only)')
      .getAttribute('type'),
    'password',
  );
});

test('the header names the account and carries no switcher of its own', async () => {
  const { rendered } = await renderPeople(await fixture(), 'acct:personal');

  // The rail header's avatar menu is the application's only account switcher,
  // and it sits beside this section; a second one here would repeat it.
  assert.equal(
    rendered.queryByRole('button', { name: 'Switch account' }),
    null,
  );
  // The header's right-hand slot carries the account's access state when
  // there is one to state, and never a control: nothing in it is a button.
  assert.equal(document.querySelector('.path .header-action button'), null);
  assert.ok(rendered.getByRole('heading', { level: 1, name: 'satoshi' }));
  // Keep the accessible verified mark brought in by the earlier UI changes.
  const serverState = rendered.getByRole('img', { name: 'Verified' });
  assert.ok(serverState.closest('.settings-inset .fr'));
  assert.equal(rendered.queryByText(/·\s*verified/i), null);
  // A connected server this Mac has not paired with reads the same way.
  assert.ok(
    rendered.getByText('Connected, not paired').classList.contains('dim'),
  );
  // The body is the panel the sub-navigation's Account tab controls.
  const panel = rendered.getByRole('tabpanel');
  assert.equal(panel.id, 'settings-sections-panel-account');
  assert.ok(ui.within(panel).getByText('Username'));
  assert.ok(rendered.getByRole('button', { name: 'Server details ›' }));
});

test('an unpaired server is hidden until its account inventory loads successfully', async () => {
  const snapshot = await fixture();
  const failed: AgentSnapshot = {
    ...snapshot,
    profileInventory: snapshot.profileInventory.map((inventory) =>
      inventory.profile === 'partner'
        ? { ...inventory, accounts: 'unavailable' }
        : inventory,
    ),
    notifications: [
      {
        id: 'catalog-partner-profile-0',
        profile: 'partner',
        severity: 'warn',
        title: 'Could not load profile overview on partner',
        detail: 'The external rollback checkpoint is missing.',
        action: 'Inspect',
      },
    ],
  };
  const { rendered } = await renderPeople(failed);

  assert.ok(rendered.getByText('Could not load profile overview on partner'));
  assert.equal(rendered.queryByText('Connected, not paired'), null);
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

test('a server note and a team roster note both route away, by the ids the agent actually sends', async () => {
  const snapshot = await fixture();
  // These are the id shapes `notificationsOf` and the roster failure path in
  // bridge.ts actually produce — distinct from the `lease-`/`team-`/`fed-`
  // shorthand the fixture's own notices use.
  const withRealIds: AgentSnapshot = {
    ...snapshot,
    notifications: [
      {
        id: 'check-in-expired-acme',
        severity: 'crit',
        title: 'Acme is locked',
        detail: 'The signed server check-in has expired.',
        action: 'Check in',
      },
      {
        id: 'group-roster-unavailable-team:eng',
        severity: 'warn',
        title: 'Team member list is unavailable',
        detail: 'The roster request did not complete.',
        action: 'Refresh',
      },
    ],
  };
  const { rendered } = await renderPeople(withRealIds, 'acct:personal');

  assert.equal(rendered.container.querySelector('.people-attention'), null);
  assert.equal(rendered.queryByText('Acme is locked'), null);
  assert.equal(rendered.queryByText('Team member list is unavailable'), null);
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
  // The band qualifies this account's facts, so it stays above both the
  // unpaired-server row and those facts rather than ahead of the account the
  // page names.
  const section = list.closest('.people-attention');
  assert.ok(section, 'the notes are drawn in the page’s notice section');
  assert.ok(
    section.closest('.account-main'),
    'the section is in the page body, under the header that names the account',
  );
  const unpaired = section.nextElementSibling;
  if (!unpaired) throw new Error('the unpaired server row is missing');
  assert.ok(unpaired.classList.contains('inset'));
  assert.match(unpaired.textContent ?? '', /Connected, not paired/);
  assert.ok(
    unpaired.nextElementSibling?.classList.contains('settings-inset'),
    'the server row precedes the account facts',
  );
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

test('a trust block stops remote workflows while local recovery controls remain enabled', async () => {
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
  assert.match(
    rename.getAttribute('title') ?? '',
    /Server verification failed/,
  );
  assert.equal(
    rendered
      .getByRole('button', { name: 'Sign in via SSO' })
      .hasAttribute('disabled'),
    true,
  );
  assert.ok(rendered.getByText('Account access is stopped'));
  assert.ok(
    rendered.getByText(
      'Remote operations are unavailable until a connection is reestablished.',
    ),
  );
  assert.ok(rendered.getByRole('button', { name: 'Open server' }));
  // Access state is a fact about the header's subject, so it is the header's
  // own right-hand mark rather than a chip trailing the server line.
  const state = document.querySelector('.path .header-action .chip');
  assert.ok(state, 'the header states the account’s access at its right');
  assert.equal(state.textContent, 'Verification failed');
  assert.equal(document.querySelector('.path .sub .chip'), null);
  // The Server row states the same thing in the quieter second phrase.
  assert.equal(
    document.querySelector('.settings-inset .fr .dim')?.textContent,
    'Verification failed',
  );
  for (const name of [
    'Bot accounts',
    'Web admin panel',
    'Import from FOKS CLI',
  ])
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
  assert.deepEqual(chosen.at(-1), {
    kind: 'settings',
    section: 'account',
    store: 'acct:personal',
  });
});

test('organization sign-in survives browser focus and can finish the same flow', async () => {
  const { rendered, readBacks } = await renderPeople(await fixture());
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Sign in via SSO' }),
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
  assert.deepEqual(readBacks, [['SSO sign-in verified', 'personal']]);
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

test('local alias closes its sheet before the read back and names one profile', async () => {
  let release = (): void => {};
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  const { rendered, readBacks } = await renderPeople(
    await fixture(),
    'acct:personal',
    () => {},
    { holdRefresh: () => held },
  );
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Change local alias' }),
  );
  const dialog = rendered.getByRole('dialog', { name: 'Change local alias' });
  ui.fireEvent.change(
    ui.within(dialog).getByRole('textbox', { name: 'Local alias' }),
    { target: { value: 'Private account' } },
  );
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Save' }));
  });
  // The sheet should close as soon as the write completes without blocking on
  // the subsequent background refresh.
  await ui.waitFor(() => assert.equal(readBacks.length, 1));
  assert.ok(!rendered.queryByRole('dialog'));
  // Updating a local alias only refreshes the affected account's server profile
  // rather than reloading the entire catalog.
  assert.deepEqual(readBacks, [['Local alias updated', 'personal']]);
  await ui.act(async () => {
    release();
    await held;
  });
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
  // The header no longer carries the alias as a chip; the Shown as row does.
  assert.ok(rendered.getAllByText('Private account').length >= 1);
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Change…' }));
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Show pending changes' }),
  );
  await ui.waitFor(() => assert.deepEqual(calls, ['personal']));
});
