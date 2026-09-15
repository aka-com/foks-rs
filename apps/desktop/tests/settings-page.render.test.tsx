/**
 * The Settings tab as one scrolling page: its sections, the `section=` address
 * that lands on one of them, the `profile=` address that opens a server, and
 * the Mac-wide reset that composes the per-server one.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge } from '../src/bridge';
import type { Location, SettingsSection } from '../src/location';
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

interface SettingsOptions {
  /** The `section=`, `profile=` and `store=` the address carries. */
  where?: { section?: SettingsSection; profile?: string; store?: string };
  /** The named fixture scene the tab was entered at. */
  scene?: string;
  onNavigate?: (location: Location) => void;
  /** Wraps the mock bridge, for a test that watches or fails one command. */
  decorate?: (bridge: Bridge) => Bridge;
  /** Collects the failures a test expects, instead of raising them. */
  onMutationError?: (error: unknown) => void;
}

async function renderSettings(
  snapshot: AgentSnapshot,
  {
    where = {},
    scene = 'settings',
    onNavigate = () => {},
    decorate = (bridge) => bridge,
    onMutationError = (error) => {
      throw error;
    },
  }: SettingsOptions = {},
) {
  const { SettingsScreen } = (await vite.ssrLoadModule(
    '/src/screens/settings-screen.tsx',
  )) as typeof import('../src/screens/settings-screen');
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
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(SettingsScreen, {
          snapshot,
          bridge: decorate(mockBridge(snapshot)),
          location: { kind: 'settings', ...where },
          scene,
          onNavigate,
          onRefresh: async () => {},
          onRefreshSnapshot: async () => snapshot,
          onError: (error: unknown) => {
            throw error;
          },
          onMutationError: async (error: unknown) => {
            onMutationError(error);
          },
          onLock: async () => true,
          agentLifecycle: { state: 'ready' },
          onRetryAgent: async () => {},
        }),
      }),
    }),
  );
  await ui.act(async () => {
    await Promise.resolve();
  });
  return rendered;
}

test('the page holds servers, credentials, about, this Mac and the danger zone', async () => {
  const rendered = await renderSettings(await fixture());

  for (const label of [
    'Account',
    'Security key credentials · satoshi on foks.example.net',
    'About',
    'This Mac',
    'Danger zone',
  ])
    assert.ok(rendered.getAllByText(label).length, `${label} is on the page`);
  // The servers list, with its own state chips and Add a server….
  assert.ok(rendered.getByText('foks.example.net'));
  assert.ok(rendered.getByRole('button', { name: 'Add a server…' }));
  assert.ok(rendered.getAllByText('Checked').length);
  assert.ok(rendered.getByText('Not verified'));
  // One passphrase row per account, with all three actions.
  assert.equal(rendered.getAllByRole('button', { name: 'Set…' }).length, 2);
  assert.equal(rendered.getAllByRole('button', { name: 'Change…' }).length, 2);
  assert.equal(rendered.getAllByRole('button', { name: 'Verify…' }).length, 2);
  assert.ok(rendered.getByRole('button', { name: 'Lock now' }));
  assert.ok(rendered.getByRole('button', { name: 'Choose folder…' }));
  // No sub-navigation: the sections are the page.
  assert.equal(rendered.queryByRole('navigation'), null);
});

test('a section address puts the page and the keyboard on that section', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'credentials' },
  });

  // The section is a labelled region, named by the label the reader sees.
  await ui.waitFor(() => {
    assert.equal(
      document.activeElement,
      rendered.getByRole('region', { name: 'Account' }),
      'the Account section takes focus',
    );
  });
  // The card credentials name the account they act on.
  assert.ok(
    rendered.getByText(
      'Security key credentials · satoshi on foks.example.net',
    ),
  );
});

test('the card credentials name an account the reader can change', async () => {
  const chosen: Location[] = [];
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'credentials' },
    onNavigate: (location) => chosen.push(location),
  });

  const group = rendered.getByRole('group', {
    name: 'Security key credentials · satoshi on foks.example.net',
  });
  const accounts = ui.within(group).getAllByRole('button');
  assert.equal(accounts.length, 2);
  await ui.act(async () => {
    ui.fireEvent.click(accounts[1]);
  });
  // The switch keeps the address the page was reached by.
  assert.deepEqual(chosen.at(-1), {
    kind: 'settings',
    section: 'credentials',
    store: 'acct:work',
  });
});

test('an address naming an account this Mac lost does not claim facts about it', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'credentials', store: 'acct:nope' },
  });

  assert.ok(rendered.getByText('Account no longer available'));
  assert.equal(
    rendered.queryByText('No security key is enrolled on this account.'),
    null,
  );
});

test('the passphrase sheet opens in the mode its row names, Verify included', async () => {
  const rendered = await renderSettings(await fixture());

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getAllByRole('button', { name: 'Verify…' })[0]);
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(
    ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
    'Account passphrase',
  );
  assert.ok(ui.within(dialog).getByText('satoshi on foks.example.net'));
  assert.ok(
    ui.within(dialog).getByRole('button', { name: 'Verify passphrase' }),
  );
  // Verify asks the server about one passphrase, so there is nothing to confirm.
  assert.equal(ui.within(dialog).queryByLabelText('Confirm'), null);
});

test('a profile address opens that server instead of the page', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { profile: 'personal' },
  });

  assert.ok(rendered.getByText('Check-in'));
  // What stops if this server lapses, as two lists of its own.
  assert.ok(rendered.getByText('Accounts on this server'));
  assert.ok(rendered.getByText('Teams on this server'));
  assert.ok(rendered.getByText('Identity and trust'));
  assert.ok(
    rendered.getByRole('button', { name: 'Inspect last check response' }),
  );
  // Forgetting a server and resetting its trust are two commands, so the
  // danger zone keeps two rows.
  assert.ok(rendered.getByRole('button', { name: 'Remove local data…' }));
  assert.ok(rendered.getByRole('button', { name: 'Erase and reset…' }));
  // The page's own sections are not drawn behind a server: the danger zone
  // here is this server's, and the Mac-wide reset is not on it.
  assert.equal(rendered.queryByText('This Mac'), null);
  assert.equal(
    rendered.queryByRole('button', { name: 'Reset this Mac…' }),
    null,
  );
  assert.equal(rendered.queryByRole('button', { name: 'Lock now' }), null);
  // The audit log reads the two numbers the agent reports, not a date.
  assert.ok(rendered.getByText(/12 entries · checkpoint 4821/));
});

test('a server page captions its groups without repeating itself', async () => {
  const snapshot = await fixture();
  const rendered = await renderSettings(snapshot, {
    where: { profile: 'personal' },
  });

  // The page is already about one server, so the caption drops it and keeps
  // what the object is and the roster summary.
  assert.ok(rendered.getByText('Named group · 2 people'));
  assert.equal(rendered.queryByText(/Named group · foks.example.net/), null);

  // A roster that could not be read is a chip at the end of the row, and the
  // caption is the bare kind rather than the failure in the summary's place.
  ui.cleanup();
  const unread = await renderSettings(
    {
      ...snapshot,
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
    },
    { where: { profile: 'personal' } },
  );
  const row = [...unread.container.querySelectorAll('.fr.devrow')].find(
    (entry) => entry.textContent?.includes('Household'),
  );
  assert.ok(row, 'the server still lists the group');
  assert.equal(
    [...row.querySelectorAll('.chip')].map((chip) => chip.textContent)[0],
    'Roster unavailable',
  );
  assert.equal(row.querySelector('small')?.textContent, 'Named group');
});

test('a server holding no group says so', async () => {
  const snapshot = await fixture();
  const rendered = await renderSettings(
    {
      ...snapshot,
      stores: snapshot.stores.filter((store) => store.kind !== 'team'),
    },
    { where: { profile: 'personal' } },
  );

  assert.ok(rendered.getByText('Teams on this server'));
  assert.ok(rendered.getByText('No groups on this server'));
});

test('a lapsed server says its check-in expired and offers the check', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/index.ts',
  )) as typeof import('../src/model');
  const rendered = await renderSettings(applyLease(await fixture(), 'lapsed'), {
    where: { profile: 'acme' },
  });

  assert.ok(rendered.getByText('Check-in expired.'));
  assert.ok(rendered.getByRole('button', { name: 'Check now' }));
  assert.ok(rendered.getByText('Hidden while locked'));
});

test('a never-checked server says what a check would discover', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { profile: 'partner' },
  });

  assert.ok(rendered.getByText('Not checked yet.'));
  assert.ok(rendered.getByText('Discovered upon first verification'));
  assert.ok(rendered.getByText('Not verified yet'));
});

test('a lapsed server can be checked from its row in the list', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/index.ts',
  )) as typeof import('../src/model');
  const rendered = await renderSettings(applyLease(await fixture(), 'lapsed'));

  // The never-checked server and the lapsed one are equally stuck.
  assert.equal(rendered.getAllByRole('button', { name: 'Check' }).length, 2);
  // The state is a chip at the end of the row, beside the check.
  assert.ok(rendered.getAllByText('Check-in expired').length);
});

test('Reset this Mac asks for one typed profile name per server', async () => {
  const rendered = await renderSettings(await fixture());

  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Reset this Mac…' }),
    );
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('alertdialog'));
  await ui.waitFor(() => {
    assert.ok(ui.within(dialog).getAllByText('Discarded operations').length);
  });
  const confirms = ui.within(dialog).getAllByPlaceholderText(/to confirm$/);
  assert.equal(confirms.length, 3);
  const run = ui.within(dialog).getByRole('button', { name: 'Reset this Mac' });
  assert.equal(run.hasAttribute('disabled'), true);

  for (const [index, profile] of ['personal', 'acme', 'partner'].entries())
    await ui.act(async () => {
      ui.fireEvent.change(confirms[index], { target: { value: profile } });
    });
  assert.equal(run.hasAttribute('disabled'), false);
  // The lifetime is the one every preview reported, not a number this page
  // invented.
  assert.ok(
    ui
      .within(dialog)
      .getByText(/Each reset confirmation expires in 60 seconds/),
  );
});

test('the reset consumes each profile’s single-use confirmation token', async () => {
  const issued = new Map<string, string>();
  const spent: [string, string, string][] = [];
  const rendered = await renderSettings(await fixture(), {
    decorate: (bridge) => ({
      ...bridge,
      describeReset: async (profile) => {
        const preview = await bridge.describeReset(profile);
        issued.set(profile, preview.token);
        return preview;
      },
      resetServer: async (profile, confirmation, token) => {
        spent.push([profile, confirmation, token]);
        return bridge.resetServer(profile, confirmation, token);
      },
    }),
  });

  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Reset this Mac…' }),
    );
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('alertdialog'));
  await ui.waitFor(() => {
    assert.ok(ui.within(dialog).getAllByText('Discarded operations').length);
  });
  const confirms = ui.within(dialog).getAllByPlaceholderText(/to confirm$/);
  for (const [index, profile] of ['personal', 'acme', 'partner'].entries())
    await ui.act(async () => {
      ui.fireEvent.change(confirms[index], { target: { value: profile } });
    });
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Reset this Mac' }),
    );
  });

  assert.deepEqual(
    spent,
    ['personal', 'acme', 'partner'].map((profile) => [
      profile,
      profile,
      issued.get(profile) ?? 'missing',
    ]),
  );
});

test('a reset that fails part way reloads every preview', async () => {
  let attempts = 0;
  let previews = 0;
  const reported: unknown[] = [];
  const rendered = await renderSettings(await fixture(), {
    decorate: (bridge) => ({
      ...bridge,
      describeReset: async (profile) => {
        previews += 1;
        return bridge.describeReset(profile);
      },
      resetServer: async (profile, confirmation, token) => {
        attempts += 1;
        if (attempts === 2) throw new Error('the server refused the reset');
        return bridge.resetServer(profile, confirmation, token);
      },
    }),
    onMutationError: (error) => reported.push(error),
  });

  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Reset this Mac…' }),
    );
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('alertdialog'));
  await ui.waitFor(() => {
    assert.ok(ui.within(dialog).getAllByText('Discarded operations').length);
  });
  assert.equal(previews, 3);
  const confirms = ui.within(dialog).getAllByPlaceholderText(/to confirm$/);
  for (const [index, profile] of ['personal', 'acme', 'partner'].entries())
    await ui.act(async () => {
      ui.fireEvent.change(confirms[index], { target: { value: profile } });
    });
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Reset this Mac' }),
    );
  });

  // Every token is spent or stale once a run has started, so the sheet asks
  // for a fresh preview from every server.
  await ui.waitFor(() => {
    assert.equal(previews, 6);
  });
  // The run stopped where it failed and said so; it did not report success.
  assert.equal(reported.length, 1);
  assert.equal(attempts, 2);
});
