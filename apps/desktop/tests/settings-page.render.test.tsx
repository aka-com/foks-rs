/**
 * The Settings tab as a sub-navigation of Account, Preferences and Device.
 * Account holds the device-wide server inventory at its bottom; `profile=`
 * opens one server there without hiding the sub-navigation. Device owns the
 * Mac-wide reset, which remains separate from server removal.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useState } from 'react';
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
  /**
   * Mounts the screen the way the shell does: under the screen error boundary,
   * keyed on the location, with the sub-navigation's own moves re-addressing
   * the page.
   */
  shellKeyed?: boolean;
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
    shellKeyed = false,
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
  const boundary = (await vite.ssrLoadModule(
    '/src/app/screen-error-boundary.tsx',
  )) as typeof import('../src/app/screen-error-boundary');
  const bridge = decorate(mockBridge(snapshot));
  const controller = new ToastController();
  const page = (
    at: SettingsOptions['where'] = where,
    navigate: SettingsOptions['onNavigate'] = onNavigate,
  ) => {
    const screen = createElement(ChatInboxProvider, {
      bridge,
      snapshot,
      children: createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot,
        children: createElement(ToastProvider, {
          controller,
          children: createElement(SettingsScreen, {
            snapshot,
            bridge,
            location: { kind: 'settings', ...at },
            scene,
            onNavigate: navigate,
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
    });
    return screen;
  };
  function ShellKeyed() {
    const [at, setAt] = useState<SettingsOptions['where']>(where);
    const location: Location = { kind: 'settings', ...at };
    return createElement(boundary.ScreenErrorBoundary, {
      key: boundary.screenBoundaryKey(location),
      identity: boundary.screenIdentity(location),
      children: page(at, (next) => {
        onNavigate(next);
        if (next.kind === 'settings') setAt(next);
      }),
    });
  }
  const rendered = ui.render(shellKeyed ? createElement(ShellKeyed) : page());
  await ui.act(async () => {
    await Promise.resolve();
  });
  /** Re-address the page, as the sub-navigation does. */
  const show = async (
    at: SettingsOptions['where'],
    nextSnapshot = snapshot,
  ): Promise<void> => {
    snapshot = nextSnapshot;
    await ui.act(async () => {
      rendered.rerender(page(at));
      await Promise.resolve();
    });
  };
  return Object.assign(rendered, { show });
}

/** The form the server detail page states an expiry in. */
const expiresLong = (value: number): string =>
  new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(value * 1000));

/** Counts `describe_server_status` requests behind the mock bridge. */
function countingStatus(counter: { reads: number }) {
  return (bridge: Bridge): Bridge => ({
    ...bridge,
    describeServerStatus: async (profile, fresh) => {
      counter.reads++;
      return bridge.describeServerStatus(profile, fresh);
    },
  });
}

test('the server inventory states lease and identity from the catalog, reading no server status', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/index.ts',
  )) as typeof import('../src/model');
  const now = Math.floor(Date.now() / 1000);
  const expiresAt = now + 12 * 86_400;
  const snapshot = applyLease(await fixture(), 'fresh', 'acme', now);
  const counter = { reads: 0 };
  const rendered = await renderSettings(snapshot, {
    where: { section: 'account' },
    decorate: countingStatus(counter),
  });

  // Configured rows stay compact; lease and identity details live on the
  // server's own page.
  const row = [...rendered.container.querySelectorAll('.srow')].find((entry) =>
    entry.textContent?.includes('Acme'),
  );
  assert.ok(row);
  assert.equal(row.textContent?.includes('Valid until'), false);

  // The server's own page states the lease with the pinned identity beside it.
  await rendered.show({ profile: 'acme' });
  assert.ok(rendered.getByText(expiresLong(expiresAt)));
  assert.ok(rendered.getByText(/33 entries · Checkpoint 90417/));
  assert.equal(counter.reads, 0);
});

test('the explicit check reads the signed status fresh and states what it read', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/index.ts',
  )) as typeof import('../src/model');
  const now = Math.floor(Date.now() / 1000);
  const snapshot = applyLease(await fixture(), 'fresh', 'acme', now);
  const renewed = now + 30 * 86_400;
  const counter = { reads: 0 };
  const rendered = await renderSettings(snapshot, {
    where: { profile: 'acme' },
    decorate: (bridge) => {
      const counted = countingStatus(counter)(bridge);
      return {
        ...counted,
        // The check's read is the one that answers for a renewed lease; the
        // catalog this page is rendering still carries the previous one.
        describeServerStatus: async (profile, fresh) => {
          const status = await counted.describeServerStatus(profile, fresh);
          return status.compatibility.status === 'required'
            ? {
                ...status,
                compatibility: { ...status.compatibility, expiresAt: renewed },
                leaseExpiresAt: renewed,
              }
            : status;
        },
      };
    },
  });
  assert.equal(counter.reads, 0);

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Check' }));
  });
  await ui.waitFor(() => assert.equal(counter.reads, 1));
  await ui.waitFor(() =>
    assert.ok(rendered.getByText('Last checked now. No issues.')),
  );
  assert.ok(rendered.getByText(expiresLong(renewed)));
  // Once a new catalog arrives, its signed status supersedes the check's
  // temporary report even though the profile and pinned host are unchanged.
  const latestExpiry = renewed + 86_400;
  await rendered.show(
    { profile: 'acme' },
    {
      ...snapshot,
      servers: snapshot.servers.map((server) =>
        server.profileName === 'acme' &&
        server.compatibility.status === 'required'
          ? {
              ...server,
              compatibility: {
                ...server.compatibility,
                expiresAt: latestExpiry,
              },
            }
          : server,
      ),
    },
  );
  assert.ok(rendered.getByText(expiresLong(latestExpiry)));
  assert.equal(counter.reads, 1);
});

test('Account opens first and carries the server inventory at its bottom', async () => {
  const rendered = await renderSettings(await fixture());

  const nav = rendered.getByRole('navigation', { name: 'Settings sections' });
  const tabs = ui
    .within(nav)
    .getAllByRole('tab')
    .map((tab) => tab.textContent);
  assert.deepEqual(tabs, ['Account', 'Preferences', 'Storage']);
  const account = ui.within(nav).getByRole('tab', { name: 'Account' });
  assert.equal(account.getAttribute('aria-selected'), 'true');
  // Account is the sub-navigation's landing page. Its header names the
  // account, not the section, and its body is the panel the tab controls.
  assert.ok(rendered.getByRole('heading', { level: 1, name: 'satoshi' }));
  const panel = rendered.getByRole('tabpanel');
  assert.equal(panel.id, account.getAttribute('aria-controls'));
  assert.ok(ui.within(panel).getByText('Username'));
  const serversLabel = ui.within(panel).getByText('Servers');
  assert.ok(
    serversLabel.closest('.sec')?.classList.contains('server-list-label'),
  );
  assert.ok(rendered.getByRole('button', { name: 'Add a server…' }));
  assert.equal(rendered.queryByText('Needs attention'), null);
  assert.equal(rendered.queryByText('Configured servers'), null);
  assert.equal(rendered.queryByText('Ready'), null);
  const serverRows = [...rendered.container.querySelectorAll('.srow')];
  assert.ok(serverRows.length > 1);
  assert.ok(
    serverRows.every(
      (row) => row.parentElement === serverRows[0].parentElement,
    ),
  );
  const accountActions = panel.querySelector('.account-more');
  assert.ok(accountActions);
  assert.equal(
    [...panel.querySelectorAll('.settings-inset, .account-more')].at(-1),
    accountActions,
  );
  const usernameRow = rendered
    .getByText('Username')
    .closest<HTMLElement>('.fr');
  assert.ok(usernameRow);
  assert.deepEqual(
    ui
      .within(usernameRow)
      .getAllByRole('button')
      .map((button) => button.textContent),
    ['Change…', 'Passphrase…'],
  );
});

test('the server inventory remains available when this device has no account', async () => {
  const snapshot = await fixture();
  const rendered = await renderSettings({
    ...snapshot,
    stores: snapshot.stores.filter((store) => store.kind !== 'account'),
  });

  assert.ok(rendered.getByText('No available account on this device'));
  assert.ok(rendered.getByText('Servers'));
  assert.ok(rendered.getByRole('button', { name: 'Add a server…' }));
});

test('Add server explains the protocol and starts with example placeholders', async () => {
  const rendered = await renderSettings(await fixture());
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Add a server…' }));

  const dialog = rendered.getByRole('dialog');
  assert.ok(
    ui
      .within(dialog)
      .getByText(
        'Add FOKS protocol v019 servers here. The server is saved only after online verification succeeds.',
      ),
  );
  assert.equal(
    ui.within(dialog).getByLabelText('Server name').getAttribute('placeholder'),
    'new-foks',
  );
  assert.equal(
    ui.within(dialog).getByLabelText('Address').getAttribute('placeholder'),
    'foks.example.com',
  );
  assert.equal(
    ui
      .within(dialog)
      .getByRole('button', { name: 'Add server' })
      .hasAttribute('disabled'),
    true,
  );
});

test('Add server verifies online and opens the profile returned by the backend', async () => {
  const submitted: Array<[string, string]> = [];
  const events: string[] = [];
  const rendered = await renderSettings(await fixture(), {
    decorate: (bridge) => ({
      ...bridge,
      checkAndAddProfile: async (profile, probe) => {
        submitted.push([profile, probe]);
        return {
          profile: 'existing-endpoint',
          acceptance: 'unchanged',
          lookupName: 'foks.example.com',
          canonicalName: 'foks.example.com',
          hostId: `02${'1'.repeat(64)}`,
          chain: 8,
          epoch: 21,
        };
      },
    }),
    onRefresh: async (message) => {
      events.push(`refresh:${message}`);
    },
    onNavigate: (location) => {
      if (location.kind === 'settings' && location.profile)
        events.push(`navigate:${location.profile}`);
    },
  });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Add a server…' }));
  const dialog = rendered.getByRole('dialog');
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Server name'), {
    target: { value: 'requested-name' },
  });
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Address'), {
    target: { value: ' foks.example.com ' },
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Add server' }),
    );
    await Promise.resolve();
  });

  assert.equal(rendered.queryByRole('dialog'), null);
  assert.deepEqual(submitted, [['requested-name', 'foks.example.com']]);
  assert.deepEqual(events, [
    'refresh:Server verified and added.',
    'navigate:existing-endpoint',
  ]);
});

test('Add server keeps the sheet open and re-enables submission after verification fails', async () => {
  const failure = new Error('TLS certificate rejected');
  const errors: unknown[] = [];
  const navigations: string[] = [];
  const refreshes: string[] = [];
  let rejectVerification: ((reason: unknown) => void) | undefined;
  const verification = new Promise<never>((_resolve, reject) => {
    rejectVerification = reject;
  });
  const rendered = await renderSettings(await fixture(), {
    decorate: (bridge) => ({
      ...bridge,
      checkAndAddProfile: async () => verification,
    }),
    onMutationError: (error) => errors.push(error),
    onRefresh: async (message) => {
      refreshes.push(message);
    },
    onNavigate: (location) => {
      if (location.kind === 'settings' && location.profile)
        navigations.push(location.profile);
    },
  });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Add a server…' }));
  const dialog = rendered.getByRole('dialog');
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Server name'), {
    target: { value: 'new-foks' },
  });
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Address'), {
    target: { value: 'bad.example' },
  });
  const add = ui.within(dialog).getByRole('button', { name: 'Add server' });
  await ui.act(async () => {
    ui.fireEvent.click(add);
    await Promise.resolve();
  });
  assert.equal(add.hasAttribute('disabled'), true);
  assert.ok(rejectVerification);
  await ui.act(async () => rejectVerification?.(failure));

  assert.equal(add.hasAttribute('disabled'), false);
  assert.equal(rendered.getByRole('dialog'), dialog);
  assert.deepEqual(errors, [failure]);
  assert.deepEqual(refreshes, []);
  assert.deepEqual(navigations, []);
});

test('opening a device-wide server retains the selected account', async () => {
  const chosen: Location[] = [];
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'account', store: 'acct:work' },
    onNavigate: (location) => chosen.push(location),
  });

  ui.fireEvent.click(rendered.getAllByRole('button', { name: 'Manage ›' })[0]);
  const destination = chosen.at(-1);
  assert.equal(destination?.kind, 'settings');
  if (destination?.kind !== 'settings') return;
  assert.equal(destination.section, 'account');
  assert.equal(destination.store, 'acct:work');
  assert.ok(destination.profile);
});

test('a retired Servers address opens the list at the bottom of Account', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'servers' },
  });
  assert.equal(
    rendered
      .getByRole('tab', { name: 'Account' })
      .getAttribute('aria-selected'),
    'true',
  );
  assert.ok(rendered.getByRole('heading', { level: 1, name: 'satoshi' }));
  assert.ok(rendered.getByText('Servers'));
  assert.ok(rendered.getByText('foks.example.net'));
  assert.ok(rendered.getByRole('button', { name: 'Add a server…' }));
  // A server that answered carries no chip — its mark already reads as one —
  // and only the states worth acting on are named.
  assert.equal(rendered.queryByText('Checked'), null);
  assert.ok(document.querySelector('.smark.ok'));
  const configured = rendered.getByText('foks.example.net').closest('.srow');
  assert.ok(configured);
  assert.doesNotMatch(configured.textContent ?? '', /satoshi|teams?/i);
  assert.ok(rendered.getByText('Not verified'));
  // Other Settings pages are not mounted behind Account.
  assert.equal(
    rendered.queryByRole('button', { name: 'Change passphrase…' }),
    null,
  );
  assert.equal(rendered.queryByRole('button', { name: 'Lock now' }), null);
  assert.equal(rendered.queryByText('Danger zone'), null);
  assert.ok(rendered.getByText('Username'));
});

test('choosing a sub-navigation section replaces the page, dropping any open server', async () => {
  const chosen: Location[] = [];
  const rendered = await renderSettings(await fixture(), {
    where: { profile: 'personal' },
    onNavigate: (location) => chosen.push(location),
  });

  ui.fireEvent.click(rendered.getByRole('tab', { name: 'Storage' }));
  assert.deepEqual(chosen.at(-1), { kind: 'settings', section: 'mac' });
});

test('walking the sub-navigation by keyboard keeps focus on the section it selects', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'account' },
    shellKeyed: true,
  });
  const strip = rendered.getByRole('tablist', { name: 'Settings sections' });
  rendered.getByRole('tab', { name: 'Account' }).focus();
  await ui.act(async () => {
    ui.fireEvent.keyDown(strip, { key: 'ArrowDown' });
    await Promise.resolve();
  });
  // The shell keys the screen's boundary on the location; a section is a page
  // of this screen, so the move must not remount it and drop the focus the
  // tab strip just placed.
  const preferences = rendered.getByRole('tab', { name: 'Preferences' });
  assert.equal(preferences.getAttribute('aria-selected'), 'true');
  assert.equal(document.activeElement, preferences);
  assert.ok(rendered.getByRole('heading', { name: 'Preferences' }));
});

test('a section address opens that page, each with the sub-navigation beside it', async () => {
  for (const [section, heading] of [
    ['preferences', 'Preferences'],
    ['mac', 'Storage'],
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

test('Preferences holds desktop alert preferences and the rail color', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'preferences' },
  });

  const main = rendered.container.querySelector('.settings-main');
  assert.ok(main);
  // The page opens on a section label, with the first-label margin rule's
  // hook: it is `.settings-main`'s first child.
  assert.equal(main.firstElementChild?.className, 'sec');
  assert.deepEqual(
    [...main.querySelectorAll(':scope > .sec')].map(
      (label) => label.textContent,
    ),
    ['Appearance', 'Desktop alerts', 'Onboarding tips'],
  );
  assert.equal(rendered.queryByRole('button', { name: 'Passphrase…' }), null);
  assert.ok(
    rendered.getByRole('checkbox', {
      name: 'Enable desktop alerts on this device',
    }),
  );
});

test('Preferences remains available without an account', async () => {
  const snapshot = await fixture();
  const rendered = await renderSettings(
    {
      ...snapshot,
      stores: snapshot.stores.filter((store) => store.kind !== 'account'),
    },
    { where: { section: 'preferences' } },
  );

  assert.ok(
    rendered.getByRole('checkbox', {
      name: 'Enable desktop alerts on this device',
    }),
  );
  assert.equal(rendered.queryByRole('button', { name: 'Passphrase…' }), null);
});

test('a server address keeps the sub-navigation on screen, Account still selected', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { profile: 'personal' },
  });

  const nav = rendered.getByRole('navigation', { name: 'Settings sections' });
  assert.equal(
    ui
      .within(nav)
      .getByRole('tab', { name: 'Account' })
      .getAttribute('aria-selected'),
    'true',
  );
  // The other two sections are still one click away, not hidden behind the
  // server the reader opened.
  assert.ok(ui.within(nav).getByRole('tab', { name: 'Storage' }));
});

test('the passphrase sheet changes a configured passphrase behind a check', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'account' },
  });

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Passphrase…' }));
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  // The account in the fixture holds a passphrase, so the sheet settles on
  // the rotation without ever offering the reader a mode to pick.
  await ui.waitFor(() =>
    assert.equal(
      ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
      'Change passphrase',
    ),
  );
  assert.equal(
    ui.within(dialog).queryByRole('group', { name: 'Passphrase action' }),
    null,
  );
  assert.ok(ui.within(dialog).getByLabelText('Current'));
  assert.ok(ui.within(dialog).getByLabelText('New'));
  assert.ok(ui.within(dialog).getByLabelText('Confirm'));
  assert.ok(ui.within(dialog).getByRole('button', { name: 'Change' }));

  // The check is the only way through: nothing offers to change the
  // passphrase without it, so the current field cannot be dismissed.
  assert.equal(
    ui.within(dialog).queryByRole('button', { name: 'Forgot?' }),
    null,
  );
  assert.equal(
    ui.within(dialog).queryByText(/without checking the current one/i),
    null,
  );
});

test('the passphrase sheet stops changing once the check is rate-limited', async () => {
  const failures: unknown[] = [];
  // The bridge reports failures as command-error records, not Error
  // instances; only a record carries the code this sheet reacts to.
  const refused = {
    code: 'rate-limited',
    message: 'Passphrase checks are rate-limited.',
    retryable: true,
    ambiguous: false,
    fatal: false,
  };
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'account' },
    decorate: (bridge) => ({
      ...bridge,
      changeAccountPassphrase: async () => {
        throw refused;
      },
    }),
    onMutationError: (error) => failures.push(error),
  });

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Passphrase…' }));
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  await ui.waitFor(() => ui.within(dialog).getByLabelText('Current'));
  const fill = (label: string, value: string): void => {
    ui.fireEvent.change(ui.within(dialog).getByLabelText(label), {
      target: { value },
    });
  };
  await ui.act(async () => {
    fill('Current', 'wrong-one');
    fill('New', 'staplerbrigadefox');
    fill('Confirm', 'staplerbrigadefox');
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Change' }),
    );
    await Promise.resolve();
  });
  await ui.waitFor(() => assert.deepEqual(failures, [refused]));

  // A refused check is not routed around, so the rotation stops here rather
  // than falling back to one this device could authorize by itself.
  await ui.waitFor(() =>
    assert.ok(ui.within(dialog).getByText(/temporarily rate-limited/i)),
  );
  assert.equal(
    ui
      .within(dialog)
      .getByRole('button', { name: 'Change' })
      .hasAttribute('disabled'),
    true,
  );

  // The same code covers a merely busy server, so typing again lets the
  // reader retry instead of leaving the sheet permanently dead.
  await ui.act(async () => {
    fill('Current', 'another-try');
  });
  assert.equal(
    ui
      .within(dialog)
      .getByRole('button', { name: 'Change' })
      .hasAttribute('disabled'),
    false,
  );
});

test('the passphrase sheet sets a first passphrase when the account has none', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'account' },
    decorate: (bridge) => ({
      ...bridge,
      accountPassphraseStatus: async () => ({
        configured: false,
        generation: 0,
      }),
    }),
  });

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Passphrase…' }));
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  await ui.waitFor(() =>
    assert.equal(
      ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
      'Set passphrase',
    ),
  );
  // Nothing to check against, so there is no current field to fill.
  assert.equal(ui.within(dialog).queryByLabelText('Current'), null);
  assert.ok(ui.within(dialog).getByLabelText('Passphrase'));
  assert.ok(ui.within(dialog).getByLabelText('Confirm'));
  assert.ok(ui.within(dialog).getByRole('button', { name: 'Set' }));
});

test('a profile address opens that server instead of the page', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { profile: 'personal' },
  });

  assert.equal(rendered.queryByText('Check-in'), null);
  assert.ok(
    rendered.getByRole('heading', { level: 1, name: 'Personal server' }),
  );
  assert.equal(rendered.queryByText('Username'), null);
  assert.equal(rendered.container.querySelector('.shead'), null);
  // What stops if this server lapses, as two lists of its own.
  assert.ok(rendered.getByText('Accounts on this server'));
  assert.ok(rendered.getByText('Teams on this server'));
  assert.ok(rendered.getByText('Identity and trust'));
  assert.ok(rendered.getByText('Internal ID'));
  assert.ok(rendered.getAllByText('personal'));
  assert.ok(rendered.getByText('Address'));
  assert.ok(rendered.getAllByText('foks.example.net').length);
  assert.ok(rendered.getByRole('button', { name: 'Rename…' }));
  assert.ok(rendered.getByText('Remove server and credentials'));
  assert.equal(rendered.getAllByRole('button', { name: 'Remove…' }).length, 1);
  // The sub-navigation stays on screen — "Storage" is one of its labels —
  // but that page's own content is not drawn behind a server: the danger
  // zone here is this server's, and the Mac-wide reset is not on it.
  assert.ok(
    ui
      .within(rendered.getByRole('navigation', { name: 'Settings sections' }))
      .getByRole('tab', { name: 'Storage' }),
  );
  assert.equal(rendered.queryByRole('button', { name: 'Reset…' }), null);
  assert.equal(rendered.queryByRole('button', { name: 'Lock now' }), null);
  // The audit log reads the two numbers the agent reports, not a date.
  assert.ok(rendered.getByText(/12 entries · Checkpoint 4821/));
});

test('server removal deletes its local credentials after exact-name confirmation', async () => {
  const calls: Array<[string, string]> = [];
  const messages: string[] = [];
  const rendered = await renderSettings(await fixture(), {
    where: { profile: 'personal' },
    decorate: (bridge) => ({
      ...bridge,
      removeServerAndCredentials: async (profile, confirmation) => {
        calls.push([profile, confirmation]);
        return { profile, removed: true };
      },
    }),
    onRefresh: async (message) => {
      messages.push(message);
    },
  });

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Remove…' }));
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('alertdialog'));
  assert.ok(
    ui.within(dialog).getByText('Remove Personal server and its credentials?'),
  );
  const remove = ui
    .within(dialog)
    .getByRole('button', { name: 'Remove server and credentials' });
  assert.equal(remove.hasAttribute('disabled'), true);
  await ui.act(async () => {
    ui.fireEvent.change(
      ui
        .within(dialog)
        .getByPlaceholderText('Type server profile name to confirm'),
      { target: { value: 'personal' } },
    );
  });
  assert.equal(remove.hasAttribute('disabled'), false);
  await ui.act(async () => {
    ui.fireEvent.click(remove);
  });
  await ui.waitFor(() => assert.deepEqual(calls, [['personal', 'personal']]));
  assert.deepEqual(messages, ['Removed Personal server and its credentials']);
});

test('server removal remains available after an identity mismatch', async () => {
  const snapshot = structuredClone(await fixture());
  const server = snapshot.servers.find(
    (entry) => entry.profileName === 'personal',
  );
  assert.ok(server);
  server.trust = {
    status: 'blocked',
    error: {
      code: 'host-mismatch',
      message: 'The server identity changed.',
      retryable: false,
      ambiguous: false,
      fatal: false,
    },
  };
  const rendered = await renderSettings(snapshot, {
    where: { profile: 'personal' },
  });

  assert.ok(rendered.getByText('Server identity mismatch'));
  const actions = rendered.getAllByRole('button', { name: 'Remove…' });
  assert.equal(actions.length, 2);
  assert.equal(
    actions.some((button) => button.hasAttribute('disabled')),
    false,
  );
  await ui.act(async () => {
    ui.fireEvent.click(actions[0]);
  });
  assert.ok(await ui.waitFor(() => rendered.getByRole('alertdialog')));
});

test('the legacy server-reset scene opens unified removal', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { profile: 'personal' },
    scene: 'servers-reset',
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('alertdialog'));
  assert.ok(
    ui.within(dialog).getByText('Remove Personal server and its credentials?'),
  );
  assert.equal(ui.within(dialog).queryByText(/Erase|reset preview/i), null);
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

test('duplicate server labels retain addresses without profile-name suffixes', async () => {
  const snapshot = await fixture();
  const duplicated = {
    ...snapshot,
    servers: snapshot.servers.map((server, index) =>
      index < 2 ? { ...server, displayLabel: 'Shared' } : server,
    ),
  };
  const rendered = await renderSettings(duplicated, {
    where: { section: 'account' },
  });
  assert.equal(rendered.getAllByText('Shared').length, 2);
  assert.equal(
    Boolean(rendered.container.querySelector('.srow .t > b em')),
    false,
  );
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
  const rendered = await renderSettings(applyLease(await fixture(), 'lapsed'), {
    where: { section: 'account' },
  });

  // The never-checked server and the lapsed one are equally stuck.
  assert.equal(rendered.getAllByRole('button', { name: 'Check' }).length, 2);
  // The state is a chip at the end of the row, beside the check.
  assert.ok(rendered.getAllByText('Check-in expired').length);
});

test('Device displays application, agent, and data sections above local reset', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'mac' },
  });

  const main = rendered.container.querySelector('.settings-main');
  assert.ok(main);
  assert.deepEqual(
    [...main.querySelectorAll(':scope > .sec')].map(
      (label) => label.textContent,
    ),
    ['Application', 'Agent', 'FOKS data', 'Danger zone'],
  );
  assert.ok(rendered.getByText(/^FOKS Desktop /));
  assert.ok(rendered.getByRole('button', { name: 'Lock now' }));
  assert.ok(rendered.getByRole('button', { name: 'Copy' }));
  assert.ok(rendered.getByRole('button', { name: 'Export…' }));
  assert.ok(rendered.getByRole('button', { name: 'Import…' }));
  assert.ok(rendered.getByRole('button', { name: 'Verify online' }));
  assert.ok(rendered.getByRole('button', { name: 'Choose folder…' }));
  assert.ok(rendered.getByRole('button', { name: 'Reset…' }));
  assert.ok(
    rendered.getByText(
      'Deletes local account keys, trust history, cached state, and pending operations.',
    ),
  );
  // While disconnected, the Status row is multi-line and top-aligned. When
  // connected, it contains only the status value.
  const status = rendered.getByText('Status').closest('.fr');
  assert.ok(status);
  assert.equal(status.querySelector('.v small'), null);
  const agentInset = status.closest('.settings-inset');
  assert.ok(agentInset);
  assert.equal(agentInset.classList.contains('middle'), false);
  // The one-line Socket row in the same inset centers itself instead.
  const socket = rendered.getByText('Socket').closest('.fr');
  assert.ok(socket);
  assert.equal(socket.classList.contains('line'), true);
  assert.equal(socket.querySelector('.v small'), null);
});

test('Reset this device asks for one typed profile name per server', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'mac' },
  });

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Reset…' }));
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
    where: { section: 'mac' },
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
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Reset…' }));
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

test('one server whose preview fails does not hold the reset of the others', async () => {
  let acmeAnswers = false;
  const spent: string[] = [];
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'mac' },
    decorate: (bridge) => ({
      ...bridge,
      describeReset: async (profile) => {
        if (profile === 'acme' && !acmeAnswers)
          throw {
            code: 'operation-failed',
            message:
              'Native credential service failed: An invalid record was encountered.',
            retryable: true,
            fatal: false,
            ambiguous: false,
          };
        return bridge.describeReset(profile);
      },
      resetServer: async (profile, confirmation, token) => {
        spent.push(profile);
        return bridge.resetServer(profile, confirmation, token);
      },
    }),
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Reset…' }));
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('alertdialog'));
  await ui.waitFor(() => {
    assert.ok(ui.within(dialog).getByText(/Not every server answered/));
  });
  // The failed server says so and is left out; the run is the other two.
  assert.ok(ui.within(dialog).getByText(/An invalid record was encountered/));
  assert.ok(ui.within(dialog).getByText(/Not included in this reset/));
  const run = ui
    .within(dialog)
    .getByRole('button', { name: 'Reset 2 of 3 servers' });
  assert.equal(run.hasAttribute('disabled'), true);
  const confirms = ui.within(dialog).getAllByPlaceholderText(/to confirm$/);
  assert.equal(confirms.length, 3);
  assert.equal((confirms[1] as HTMLInputElement).disabled, true);
  for (const [index, profile] of [
    [0, 'personal'],
    [2, 'partner'],
  ] as const)
    await ui.act(async () => {
      ui.fireEvent.change(confirms[index], { target: { value: profile } });
    });
  assert.equal(run.hasAttribute('disabled'), false);

  // Its own retry brings the failed server back into the run.
  acmeAnswers = true;
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Retry preview' }),
    );
  });
  await ui.waitFor(() =>
    assert.ok(
      ui.within(dialog).getByRole('button', { name: 'Reset this device' }),
    ),
  );
  assert.equal(
    ui.within(dialog).queryByText(/Not every server answered/),
    null,
  );
  await ui.act(async () => {
    ui.fireEvent.change(confirms[1], { target: { value: 'acme' } });
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Reset this device' }),
    );
  });
  assert.deepEqual(spent, ['personal', 'acme', 'partner']);
});

test('a preview that could not read this Mac’s credentials still authorizes the reset', async () => {
  const spent: string[] = [];
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'mac' },
    decorate: (bridge) => ({
      ...bridge,
      describeReset: async (profile) => {
        const preview = await bridge.describeReset(profile);
        return profile === 'personal'
          ? {
              ...preview,
              resumables: [],
              credentialsUnavailable:
                'Native credential service failed: An invalid record was encountered.',
            }
          : preview;
      },
      resetServer: async (profile, confirmation, token) => {
        spent.push(profile);
        return bridge.resetServer(profile, confirmation, token);
      },
    }),
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Reset…' }));
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('alertdialog'));
  await ui.waitFor(() => {
    assert.ok(ui.within(dialog).getByText(/An invalid record was encountered/));
  });
  // The row says what the reset will and will not reach, and the server is
  // in the run like any other.
  assert.ok(
    ui.within(dialog).getByText(/leaves the credential records it cannot read/),
  );
  assert.equal(ui.within(dialog).queryByText(/Not included/), null);
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
  assert.deepEqual(spent, ['personal', 'acme', 'partner']);
});

test('a reset that fails part way reloads every preview', async () => {
  let attempts = 0;
  let previews = 0;
  const reported: unknown[] = [];
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'mac' },
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
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Reset…' }));
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
    where: { section: 'preferences' },
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
  // Desktop alerts is a section of the Preferences page, not an anchor
  // scrolled to inside a longer one, so nothing forces focus onto it.
  assert.ok(rendered.getByRole('tabpanel', { name: 'Preferences' }));
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
    where: { section: 'account', profile: 'personal' },
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

test('Device offers to restart the agent, and names the process it would stop', async () => {
  const snapshot = await fixture();
  const calls: boolean[] = [];
  const rendered = await renderSettings(snapshot, {
    where: { section: 'mac' },
    decorate: (bridge) => ({
      ...bridge,
      restartAgent: async (takeover) => {
        calls.push(takeover);
        return { state: 'idle', generation: 1, revision: 0 };
      },
    }),
  });
  const status = rendered.getByText('Status').closest<HTMLElement>('.fr');
  assert.ok(status);
  assert.equal(status.querySelector('.v small'), null);
  assert.equal(rendered.queryByText('Restart'), null);

  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(status).getByRole('button', { name: 'Restart…' }),
    );
  });
  const sheet = document.querySelector<HTMLElement>('.sheet');
  assert.ok(sheet);
  assert.equal(sheet.querySelector('h2')?.textContent, 'Restart the agent?');
  assert.match(
    sheet.textContent ?? '',
    /FOKS will stop the local agent and start it again\. This will take a few seconds\./,
  );
  await ui.waitFor(() =>
    assert.match(sheet.textContent ?? '', /foks-agent \(PID 50350\) · Started/),
  );
  assert.match(sheet.textContent ?? '', /Nothing in progress\./);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Restart agent' }),
    );
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  assert.equal(document.querySelector('.sheet'), null);
  // An agent we own needs no takeover.
  assert.deepEqual(calls, [false]);
});

test('a maintenance action refused for a foreign agent asks to restart it, then runs', async () => {
  const snapshot = await fixture();
  const log: string[] = [];
  let refused = true;
  const rendered = await renderSettings(snapshot, {
    where: { section: 'mac' },
    decorate: (bridge) => ({
      ...bridge,
      agentProcessInfo: async () => ({
        pid: 4242,
        executable: '/opt/foks/foks-agent',
        startedAt: Math.floor(Date.now() / 1000) - 86_400,
        owned: false,
      }),
      maintainClientState: async (action) => {
        log.push(`maintain:${action}`);
        // The shape the native bridge delivers for a refused maintenance.
        if (refused)
          throw {
            code: 'external-agent',
            message: 'The running foks-agent was not started by this app.',
            retryable: false,
            ambiguous: false,
            fatal: false,
          };
        return { state: 'idle', generation: 2, revision: 0 };
      },
      restartAgent: async (takeover) => {
        log.push(`restart:${takeover}`);
        refused = false;
        return { state: 'idle', generation: 1, revision: 0 };
      },
    }),
  });
  await ui.waitFor(() =>
    assert.match(
      rendered.getByText('Status').closest('.fr')?.textContent ?? '',
      /Agent started by another app\. Use 'Restart\.\.\.' to take ownership of the agent\./,
    ),
  );
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Export…' }));
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  const sheet = document.querySelector<HTMLElement>('.sheet');
  assert.ok(sheet);
  assert.equal(
    sheet.querySelector('h2')?.textContent,
    'Restart the agent to export?',
  );
  assert.match(
    sheet.textContent ?? '',
    /The running foks-agent was not started by this app\. Stop it, and start a new agent, to enable export\?/,
  );
  assert.match(
    sheet.textContent ?? '',
    /foks-agent \(PID 4242\) · Started yesterday/,
  );
  assert.match(sheet.textContent ?? '', /\/opt\/foks\/foks-agent/);
  // No "In progress" row when the restart serves an action.
  assert.doesNotMatch(sheet.textContent ?? '', /Nothing in progress/);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Restart agent' }),
    );
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  assert.deepEqual(log, ['maintain:export', 'restart:true', 'maintain:export']);
  assert.equal(document.querySelector('.sheet'), null);
});

test('a passphrase conflict blocks another write until its status refresh succeeds', async () => {
  let reads = 0;
  let writes = 0;
  let rejectStatus!: (error: Error) => void;
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'account' },
    onMutationError: () => {},
    decorate: (bridge) => ({
      ...bridge,
      accountPassphraseStatus: async () => {
        reads++;
        if (reads === 2)
          return new Promise((_resolve, reject) => {
            rejectStatus = reject;
          });
        return { configured: true, generation: reads };
      },
      changeAccountPassphrase: async () => {
        writes++;
        throw {
          code: 'conflict',
          message: 'Passphrase changed',
          retryable: false,
          ambiguous: false,
          fatal: false,
        };
      },
    }),
  });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Passphrase…' }));
  const dialog = await rendered.findByRole('dialog');
  await ui.waitFor(() => ui.within(dialog).getByLabelText('Current'));
  for (const [label, value] of [
    ['Current', 'old passphrase'],
    ['New', 'new passphrase'],
    ['Confirm', 'new passphrase'],
  ]) {
    ui.fireEvent.change(ui.within(dialog).getByLabelText(label), {
      target: { value },
    });
  }
  const change = () =>
    ui
      .within(dialog)
      .getByRole<HTMLButtonElement>('button', { name: 'Change' });
  ui.fireEvent.click(change());
  await ui.waitFor(() => assert.equal(reads, 2));
  assert.equal(change().disabled, true);
  await ui.act(async () => rejectStatus(new Error('Status unavailable')));
  await ui.waitFor(() =>
    assert.ok(ui.within(dialog).getByText('Passphrase state unavailable')),
  );
  assert.equal(change().disabled, true);
  ui.fireEvent.click(change());
  assert.equal(writes, 1);
  ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Retry' }));
  await ui.waitFor(() => assert.equal(change().disabled, false));
  assert.equal(reads, 3);
});

test('Preferences applies and persists the chosen appearance', async () => {
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'preferences' },
  });
  try {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Dark' }));
    assert.equal(document.documentElement.dataset.theme, 'dark');
    assert.equal(window.localStorage.getItem('appearance'), 'dark');
    assert.equal(
      rendered
        .getByRole('button', { name: 'Dark' })
        .getAttribute('aria-pressed'),
      'true',
    );
    ui.fireEvent.click(rendered.getByRole('button', { name: 'System' }));
    assert.equal(window.localStorage.getItem('appearance'), 'system');
  } finally {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Light' }));
  }
});

test('dismissed onboarding tips persist and can be restored in Preferences', async () => {
  const key = 'onboarding.dismissed.add-password';
  window.localStorage.setItem(key, 'true');
  const options: SettingsOptions = { where: { section: 'preferences' } };
  const first = await renderSettings(await fixture(), options);
  const checkbox = first.getByRole('checkbox', {
    name: 'Show the add a password tip',
  }) as HTMLInputElement;
  assert.equal(checkbox.checked, false);
  ui.fireEvent.click(first.getByText('Add a password'));
  assert.equal(checkbox.checked, true);
  assert.equal(window.localStorage.getItem(key), null);
  ui.fireEvent.click(checkbox);
  assert.equal(window.localStorage.getItem(key), 'true');
  first.unmount();
  const second = await renderSettings(await fixture(), options);
  assert.equal(
    (
      second.getByRole('checkbox', {
        name: 'Show the add a password tip',
      }) as HTMLInputElement
    ).checked,
    false,
  );
  window.localStorage.removeItem(key);
});

test('Restart waits for process information and offers retry after a failed read', async () => {
  let reject!: (error: Error) => void;
  let resolve!: (info: Awaited<ReturnType<Bridge['agentProcessInfo']>>) => void;
  const rendered = await renderSettings(await fixture(), {
    where: { section: 'mac' },
    decorate: (bridge) => ({
      ...bridge,
      agentProcessInfo: () =>
        new Promise((done, fail) => {
          resolve = done;
          reject = fail;
        }),
    }),
  });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Restart…' }));
  assert.ok(rendered.getByText('Loading agent information…'));
  assert.equal(
    rendered.queryByText('No agent is answering on the socket.'),
    null,
  );
  await ui.act(async () => reject(new Error('Information unavailable')));
  assert.equal(
    rendered.queryByText('No agent is answering on the socket.'),
    null,
  );
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Retry' }));
  assert.ok(rendered.getByText('Loading agent information…'));
  await ui.act(async () =>
    resolve({ pid: null, owned: false, executable: null, startedAt: null }),
  );
  assert.ok(rendered.getByText('No agent is answering on the socket.'));
});

test('server details wait for account and team listings before claiming either is empty', async () => {
  const snapshot = await fixture();
  const rendered = await renderSettings(
    {
      ...snapshot,
      stores: [],
      accounts: [],
      items: [],
      profileInventory: snapshot.profileInventory.map((entry) => ({
        ...entry,
        accounts: 'unavailable',
        teams: 'unavailable',
      })),
    },
    { where: { profile: 'personal' } },
  );
  assert.ok(rendered.getAllByText('Loading accounts…').length);
  assert.ok(rendered.getByText('Loading teams…'));
  assert.equal(rendered.queryByText('No account on this server'), null);
  assert.equal(rendered.queryByText('No teams on this server'), null);
});
