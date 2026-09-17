/**
 * The Settings tab as a sub-navigation of six pages: the `section=` address
 * that opens one of them, the `profile=` address that opens a server on the
 * Servers page without hiding the sub-navigation, and the Mac-wide reset that
 * composes the per-server one, now on the This device page.
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
  onRefresh?: (message: string) => Promise<void>;
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
    onRefresh = async () => {},
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
  const { ChatInboxProvider } = await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
  );
  const bridge = decorate(mockBridge(snapshot));
  const rendered = ui.render(
    createElement(ChatInboxProvider, {
      bridge,
      snapshot,
      children: createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot,
        children: createElement(ToastProvider, {
          controller: new ToastController(),
          children: createElement(SettingsScreen, {
            snapshot,
            bridge,
            location: { kind: 'settings', ...where },
            scene,
            onNavigate,
            onRefresh,
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
    }),
  );
  await ui.act(async () => {
    await Promise.resolve();
  });
  return rendered;
}

test('the sub-navigation lists every section, and Servers opens first', async () => {
  const rendered = await renderSettings(await fixture());

  const nav = rendered.getByRole('navigation', { name: 'Settings sections' });
  const tabs = ui
    .within(nav)
    .getAllByRole('tab')
    .map((tab) => tab.textContent);
  assert.deepEqual(tabs, [
    'Servers',
    'Account',
    'Security keys',
    'Notifications',
    'This device',
    'About',
  ]);
  assert.equal(
    ui
      .within(nav)
      .getByRole('tab', { name: 'Servers' })
      .getAttribute('aria-selected'),
    'true',
  );
  // Servers is the sub-navigation's landing page: its list, with its own
  // state chips and Add a server…, not a section scrolled past on the way to
  // it.
  assert.ok(rendered.getByRole('heading', { level: 1, name: 'Servers' }));
  assert.ok(rendered.getByText('foks.example.net'));
  assert.ok(rendered.getByRole('button', { name: 'Add a server…' }));
  assert.ok(rendered.getAllByText('Checked').length);
  assert.ok(rendered.getByText('Not verified'));
  // Only one page is mounted at a time: the other five sections' own content
  // is not drawn behind Servers.
  assert.equal(rendered.queryByRole('button', { name: 'Set…' }), null);
  assert.equal(rendered.queryByRole('button', { name: 'Lock now' }), null);
  assert.equal(rendered.queryByText('Danger zone'), null);
});

test('choosing a sub-navigation section replaces the page, dropping any open server', async () => {
  const chosen: Location[] = [];
  const rendered = await renderSettings(await fixture(), {
    where: { profile: 'personal' },
    onNavigate: (location) => chosen.push(location),
  });

  ui.fireEvent.click(rendered.getByRole('tab', { name: 'This device' }));
  assert.deepEqual(chosen.at(-1), { kind: 'settings', section: 'device' });
});

test('a section address opens that page, each with the sub-navigation beside it', async () => {
  for (const [section, heading] of [
    ['credentials', 'Account'],
    ['security-keys', 'Security keys'],
    ['notifications', 'Notifications'],
    ['device', 'This device'],
    ['about', 'About'],
  ] as const) {
    ui.cleanup();
    const rendered = await renderSettings(await fixture(), {
      where: { section },
    });
    assert.ok(
      rendered.getByRole('heading', { level: 1, name: heading }),
      `${section} opens on its own page`,
    );
    assert.equal(
      rendered
        .getByRole('navigation', { name: 'Settings sections' })
        .querySelector('.tab.on')?.textContent,
      heading,
      `${section}'s own tab reads on`,
    );
  }
});

test('Settings links to profile-scoped security key management', async () => {
  const chosen: Location[] = [];
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'security-keys' },
    onNavigate: (location) => chosen.push(location),
  });
  ui.fireEvent.click(
    rendered.getAllByRole('button', { name: 'Open security keys' })[0],
  );
  assert.deepEqual(chosen.at(-1), {
    kind: 'settings',
    section: 'servers',
    profile: 'personal',
  });
  assert.equal(rendered.queryByRole('button', { name: 'Change PIN…' }), null);
});

test('an address naming an account this device lost does not claim facts about its keys', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'security-keys', store: 'acct:nope' },
  });

  assert.ok(rendered.getByText('Account no longer available'));
  assert.equal(
    rendered.queryByText('No security key is enrolled on this account.'),
    null,
  );
});

test('a server address keeps the sub-navigation on screen, Servers still selected', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { profile: 'personal' },
  });

  const nav = rendered.getByRole('navigation', { name: 'Settings sections' });
  assert.equal(
    ui
      .within(nav)
      .getByRole('tab', { name: 'Servers' })
      .getAttribute('aria-selected'),
    'true',
  );
  // The other five sections are still one click away, not hidden behind the
  // server the reader opened.
  assert.ok(ui.within(nav).getByRole('tab', { name: 'This device' }));
});

test('a Security keys address with no servers on this device says so', async () => {
  const snapshot = await fixture();
  const rendered = await renderSettings(
    { ...snapshot, servers: [] },
    { where: { section: 'security-keys' } },
  );

  assert.ok(rendered.getByText('No servers on this device yet.'));
});

test('the passphrase sheet opens in the mode its row names, Verify included', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'credentials' },
  });

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getAllByRole('button', { name: 'Verify…' })[0]);
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(
    ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
    'Account passphrase',
  );
  assert.equal(dialog.querySelector('.hd small'), null);
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
  assert.ok(rendered.getByText('Profile'));
  assert.ok(rendered.getAllByText('personal'));
  assert.ok(rendered.getByText('Address'));
  assert.ok(rendered.getAllByText('foks.example.net').length);
  assert.ok(rendered.getByRole('button', { name: 'Rename…' }));
  assert.ok(
    rendered.getByRole('button', { name: 'Inspect last check response' }),
  );
  // Forgetting a server and resetting its trust are two commands, so the
  // danger zone keeps two rows.
  assert.ok(rendered.getByRole('button', { name: 'Remove local data…' }));
  assert.ok(rendered.getByRole('button', { name: 'Erase and reset…' }));
  // The sub-navigation stays on screen — "This device" is one of its labels
  // now — but that page's own content is not drawn behind a server: the
  // danger zone here is this server's, and the Mac-wide reset is not on it.
  assert.ok(
    ui
      .within(rendered.getByRole('navigation', { name: 'Settings sections' }))
      .getByRole('tab', { name: 'This device' }),
  );
  assert.equal(
    rendered.queryByRole('button', { name: 'Reset this device…' }),
    null,
  );
  assert.equal(rendered.queryByRole('button', { name: 'Lock now' }), null);
  // The audit log reads the two numbers the agent reports, not a date.
  assert.ok(rendered.getByText(/12 entries · checkpoint 4821/));
});

test('server display names can be set and cleared through the stable profile id', async () => {
  const calls: Array<[string, string | null]> = [];
  const messages: string[] = [];
  const rendered = await renderSettings(await fixture(), {
    where: { profile: 'personal' },
    decorate: (bridge) => ({
      ...bridge,
      setServerLabel: async (profile, label) => {
        calls.push([profile, label]);
        return { profile, label, changed: true };
      },
    }),
    onRefresh: async (message) => {
      messages.push(message);
    },
  });

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Rename…' }));
  });
  let dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(dialog.querySelector('.hd small'), null);
  const field = ui.within(dialog).getByLabelText('Display name');
  assert.equal((field as HTMLInputElement).value, 'Personal server');
  await ui.act(async () => {
    ui.fireEvent.change(field, { target: { value: '  FOKS  ' } });
  });
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Save' }));
  });
  await ui.waitFor(() => assert.equal(rendered.queryByRole('dialog'), null));
  assert.deepEqual(calls, [['personal', 'FOKS']]);
  assert.deepEqual(messages, ['Server name updated.']);

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Rename…' }));
  });
  dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  const clearing = ui.within(dialog).getByLabelText('Display name');
  await ui.act(async () => {
    ui.fireEvent.change(clearing, { target: { value: '   ' } });
  });
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Save' }));
  });
  await ui.waitFor(() => assert.equal(rendered.queryByRole('dialog'), null));
  assert.deepEqual(calls.at(-1), ['personal', null]);
});

test('a rename failure leaves the sheet open and reports the error', async () => {
  const failures: unknown[] = [];
  const expected = new Error('label write failed');
  const rendered = await renderSettings(await fixture(), {
    where: { profile: 'personal' },
    decorate: (bridge) => ({
      ...bridge,
      setServerLabel: async () => Promise.reject(expected),
    }),
    onMutationError: (error) => failures.push(error),
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Rename…' }));
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Save' }));
    await Promise.resolve();
  });
  await ui.waitFor(() => assert.deepEqual(failures, [expected]));
  assert.equal(rendered.getByRole('dialog'), dialog);
});

test('duplicate server labels retain visible profile-name disambiguation', async () => {
  const snapshot = await fixture();
  const duplicated = {
    ...snapshot,
    servers: snapshot.servers.map((server, index) =>
      index < 2 ? { ...server, label: 'Shared' } : server,
    ),
  };
  const rendered = await renderSettings(duplicated);
  assert.equal(rendered.getAllByText('Shared').length, 2);
  assert.ok(rendered.getByText('foks.example.net'));
  assert.ok(rendered.getByText('foks.acme-corp.com'));
});

test('a server page captions its groups without repeating itself', async () => {
  const snapshot = await fixture();
  const rendered = await renderSettings(snapshot, {
    where: { profile: 'personal' },
  });

  // The page is already about one server, so the caption drops it and keeps
  // what the object is and the roster summary.
  assert.ok(rendered.getByText('Named team · 2 people'));
  assert.equal(rendered.queryByText(/Named team · foks.example.net/), null);

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
  assert.equal(row.querySelector('small')?.textContent, 'Named team');
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
  assert.ok(rendered.getByText('No teams on this server'));
});

test('a lapsed server says its check-in expired and offers the check', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/index.ts',
  )) as typeof import('../src/model');
  const rendered = await renderSettings(applyLease(await fixture(), 'lapsed'), {
    where: { profile: 'acme' },
  });

  // The band leads with the condition, which the row's chip also names.
  assert.ok(
    [...document.querySelectorAll('.band b')].some(
      (node) => node.textContent === 'Check-in expired',
    ),
  );
  assert.ok(rendered.getByRole('button', { name: 'Check now' }));
  assert.ok(rendered.getByText('Hidden while locked'));
});

test('a never-checked server says what a check would discover', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { profile: 'partner' },
  });

  assert.ok(rendered.getByText('Not checked yet'));
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

test('Reset this device asks for one typed profile name per server', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'device' },
  });

  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Reset this device…' }),
    );
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('alertdialog'));
  await ui.waitFor(() => {
    assert.ok(ui.within(dialog).getAllByText('Discarded operations').length);
  });
  const confirms = ui.within(dialog).getAllByPlaceholderText(/to confirm$/);
  assert.equal(confirms.length, 3);
  const run = ui
    .within(dialog)
    .getByRole('button', { name: 'Reset this device' });
  assert.equal(run.hasAttribute('disabled'), true);

  for (const [index, profile] of ['personal', 'acme', 'partner'].entries())
    await ui.act(async () => {
      ui.fireEvent.change(confirms[index], { target: { value: profile } });
    });
  assert.equal(run.hasAttribute('disabled'), false);
  // The lifetime is the one every preview reported, not a number this page
  // invented.
  assert.ok(ui.within(dialog).getByText(/Confirmations expire in 60 seconds/));
});

test('the reset consumes each profile’s single-use confirmation token', async () => {
  const issued = new Map<string, string>();
  const spent: [string, string, string][] = [];
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'device' },
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
      rendered.getByRole('button', { name: 'Reset this device…' }),
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
      ui.within(dialog).getByRole('button', { name: 'Reset this device' }),
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
    where: { section: 'device' },
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
      rendered.getByRole('button', { name: 'Reset this device…' }),
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
      ui.within(dialog).getByRole('button', { name: 'Reset this device' }),
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

test('device notification preferences are reachable without a channel and recover denied permission', async () => {
  let configurations = 0;
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'notifications' },
    decorate: (base) => ({
      ...base,
      chatLocal: async (action) => {
        if (action.action === 'configure') {
          configurations++;
          throw new Error('Permission denied for test');
        }
        return {
          epoch: 'aa'.repeat(16),
          available: true,
          settings: { enabled: false, previews: false, overrides: {} },
        };
      },
    }),
  });
  // Notifications is the sub-navigation's own page now, not an anchor
  // scrolled to inside a longer one, so nothing forces focus onto it.
  assert.ok(rendered.getByRole('tabpanel', { name: 'Notifications' }));
  const enable = rendered.getByRole('checkbox', {
    name: 'Enable desktop alerts on this device',
  }) as HTMLInputElement;
  ui.fireEvent.click(enable);
  await rendered.findByText('Permission denied for test');
  await ui.waitFor(() => assert.equal(enable.disabled, false));
  assert.equal(enable.checked, false);
  assert.equal(configurations, 1);
  assert.equal(rendered.queryByLabelText('Channel alerts'), null);
});

test('server security-key actions name the selected enrollment across accounts', async () => {
  const calls: unknown[] = [];
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'servers', profile: 'personal' },
    decorate: (base) => ({
      ...base,
      listYubiAccounts: async () => [
        { alias: 'first', state: 'complete' },
        { alias: 'travel', state: 'complete' },
      ],
      runYubi: async (command) => {
        calls.push(command);
        return { applied: true };
      },
    }),
  });
  await rendered.findByText('travel');
  const row = rendered.getByText('travel').parentElement;
  assert.ok(row);
  ui.fireEvent.click(
    ui.within(row).getByRole('button', { name: 'Change PIN…' }),
  );
  const dialog = rendered.getByRole('dialog');
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Card PIN'), {
    target: { value: '123456' },
  });
  ui.fireEvent.change(ui.within(dialog).getByLabelText('New PIN'), {
    target: { value: '654321' },
  });
  ui.fireEvent.click(
    ui.within(dialog).getByRole('button', { name: 'Continue' }),
  );
  await ui.waitFor(() => assert.equal(calls.length, 1));
  assert.deepEqual(calls[0], {
    command: 'change_yubi_pin',
    args: {
      profile: 'personal',
      alias: 'travel',
      oldPin: '123456',
      newPin: '654321',
    },
  });
});
