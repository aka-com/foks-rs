/**
 * The Devices tab: Macs, recovery phrases and security keys in one list on one
 * page, the chooser that adds one, the recovery operations grouped on a
 * security key's own page, and the scenes that still open a sheet on
 * arrival.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge } from '../src/bridge';
import type { Location, NavigateOptions } from '../src/location';
import type { StoreRef, AgentSnapshot } from '../src/model';
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

interface DevicesOptions {
  /** The account the address names, if any. */
  store?: StoreRef;
  /** The named fixture scene the tab was entered at. */
  scene?: string;
  /** A `section=` address written before the page was one. */
  section?: 'macs' | 'keys';
  /** One key's own page: the id of a Mac or recovery phrase, or `yubi:<alias>`. */
  device?: string;
  onNavigate?: (location: Location, options?: NavigateOptions) => void;
  /** Wraps the mock bridge, for a test that watches one command. */
  decorate?: (bridge: Bridge) => Bridge;
  /** Where a failed read lands; by default a failure fails the test. */
  onError?: (error: unknown) => void;
  /** The shell's catalog refresh, for a test that watches whether it runs. */
  refresh?: () => Promise<AgentSnapshot>;
  /** The bar's search text. */
  query?: string;
}

async function renderDevices(
  snapshot: AgentSnapshot,
  {
    store,
    scene = 'devices',
    section,
    device,
    onNavigate = () => {},
    decorate = (bridge) => bridge,
    onError = (error: unknown) => {
      throw error;
    },
    refresh = async () => snapshot,
    query,
  }: DevicesOptions = {},
) {
  const { DevicesScreen } = (await vite.ssrLoadModule(
    '/src/screens/devices-screen.tsx',
  )) as typeof import('../src/screens/devices-screen');
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
  const bridge = decorate(mockBridge(snapshot));
  const controller = new ToastController();
  const page = (at: StoreRef | undefined) =>
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller,
        children: createElement(DevicesScreen, {
          snapshot,
          bridge,
          location: {
            kind: 'devices',
            ...(at ? { store: at } : {}),
            ...(section ? { section } : {}),
            ...(device ? { device } : {}),
          },
          scene,
          onNavigate,
          onRefresh: async () => {},
          onRefreshSnapshot: refresh,
          onError,
          onMutationError: async (error: unknown) => {
            throw error;
          },
          ...(query ? { query } : {}),
        }),
      }),
    });
  const rendered = ui.render(page(store));
  await ui.act(async () => {
    await Promise.resolve();
  });
  /** Re-address the page at another account, as the switcher does. */
  const showAccount = async (at: StoreRef): Promise<void> => {
    await ui.act(async () => {
      rendered.rerender(page(at));
      await Promise.resolve();
    });
  };
  return Object.assign(rendered, { showAccount });
}

test('one list groups the devices, security keys and recovery phrases', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });

  // One region, named by the list's own heading, with the account's
  // keys grouped by what they are.
  const list = rendered.getByRole('region', { name: 'Devices' });
  assert.ok(list);
  assert.equal(
    rendered.queryByText('Computers and security keys') === null,
    true,
  );
  const groups = ui
    .within(list)
    .getAllByRole('group')
    .map((group) => group.getAttribute('aria-label'));
  assert.deepEqual(groups, ['Devices', 'Security keys', 'Recovery phrases']);
  assert.equal(
    rendered.queryByRole('group', { name: 'Accounts on this device' }) === null,
    true,
  );

  // This Mac says so; no row carries a destructive action: removal and
  // revocation are on the key's details.
  assert.ok(rendered.getByText('This device'));
  assert.equal(
    rendered.queryAllByRole('button', { name: 'Remove…' }).length,
    0,
  );
  assert.equal(
    rendered.queryAllByRole('button', { name: 'Revoke…' }).length,
    0,
  );
  // The key on a card is a device on the account, under Devices, and its
  // caption names its hardware type and role.
  const devices = ui.within(list).getByRole('group', { name: 'Devices' });
  assert.ok(ui.within(devices).getByText('Pocket YubiKey'));
  assert.ok(ui.within(devices).getByText('Key on a card · Member'));
  assert.ok(ui.within(devices).getByText('Travel Mac'));
  assert.equal(ui.within(devices).getAllByText(/^Computer · /).length, 2);
  // A security key's caption carries the user-facing label and its card.
  const keys = ui.within(list).getByRole('group', { name: 'Security keys' });
  assert.ok(ui.within(keys).getByText('primary key'));
  assert.ok(ui.within(keys).getByText(/^Security key · /));
  assert.ok(ui.within(keys).getByText('Enrolled'));
  const phrases = ui
    .within(list)
    .getByRole('group', { name: 'Recovery phrases' });
  assert.ok(ui.within(phrases).getByText('paper-backup'));
  assert.ok(ui.within(phrases).getByText('Recovery phrase'));
  assert.equal(document.querySelector('.devrow .kid') === null, true);
  // Every row is an option of one listbox, in group order.
  const listbox = ui.within(list).getByRole('listbox', { name: 'Devices' });
  const names = ui
    .within(listbox)
    .getAllByRole('option')
    .map((option) => option.getAttribute('aria-label'));
  assert.deepEqual(names, [
    'MacBook Pro',
    'Travel Mac',
    'Pocket YubiKey',
    'primary key',
    'paper-backup',
  ]);
  // With no key chosen the details say so.
  assert.ok(rendered.getByText('No Selection'));
  // "Add a device" is the only add control; the hardware scan is in the
  // More menu beside it.
  assert.equal(
    rendered.queryByRole('button', { name: 'Add a recovery phrase…' }) === null,
    true,
  );
  assert.ok(rendered.getByRole('button', { name: 'Add a device' }));
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'More' }));
  });
  assert.ok(
    await ui.waitFor(() =>
      rendered.getByRole('menuitem', { name: 'Refresh Hardware Keys' }),
    ),
  );
  // Account creation and recovery live with the other account workflows.
  assert.equal(
    rendered.queryByRole('button', {
      name: 'Connect an Existing Account with a Recovery Phrase',
    }) === null,
    true,
  );
  assert.equal(
    rendered.queryByRole('button', {
      name: 'Create an Account on a Hardware Key…',
    }) === null,
    true,
  );
});

test('the search field narrows the list to the names it matches', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
    query: 'mac',
  });
  const names = rendered
    .getAllByRole('option')
    .map((option) => option.getAttribute('aria-label'));
  assert.deepEqual(names, ['MacBook Pro', 'Travel Mac']);
});

/** Open the chooser and continue on the card with this title. */
async function choose(
  rendered: Awaited<ReturnType<typeof renderDevices>>,
  title: string,
): Promise<HTMLElement> {
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Add a device' }));
  });
  const chooser = await ui.waitFor(() => rendered.getByRole('dialog'));
  const card = ui
    .within(chooser)
    .getAllByRole('radio')
    .find((choice) => choice.querySelector('b')?.textContent === title);
  assert.ok(card, `the chooser offers "${title}"`);
  await ui.act(async () => {
    ui.fireEvent.click(card);
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(chooser).getByRole('button', { name: 'Continue' }),
    );
  });
  return ui.waitFor(() => rendered.getByRole('dialog'));
}

test('the chooser offers each way to add, and leads into pairing', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Add a device' }));
  });
  const chooser = await ui.waitFor(() => rendered.getByRole('dialog'));
  const choices = ui.within(chooser).getAllByRole('radio');
  assert.deepEqual(
    choices.map((choice) => choice.querySelector('b')?.textContent),
    [
      'Pair another device',
      'Pair this device with another account',
      'Create a new recovery phrase',
      'Connect a hardware key',
    ],
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(chooser).getByRole('button', { name: 'Continue' }),
    );
  });
  const pairing = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(
    ui.within(pairing).getByRole('heading', { level: 2 }).textContent,
    'Pair Another Device',
  );
  assert.ok(ui.within(pairing).getByText('On the other device'));
  // Finish belongs to a started offer, so it waits for one.
  assert.equal(
    ui
      .within(pairing)
      .getByRole('button', { name: 'Finish' })
      .hasAttribute('disabled'),
    true,
  );
});

test('starting a pairing reveals the phrase with a way to copy it', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Add a device' }));
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Continue' }));
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Start' }));
  });
  const pairing = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.ok(ui.within(pairing).getByRole('button', { name: 'Copy' }));
  assert.equal(
    ui
      .within(pairing)
      .getByRole('button', { name: 'Finish' })
      .hasAttribute('disabled'),
    false,
  );

  ui.fireEvent(window, new Event('blur'));
  assert.ok(rendered.getByRole('dialog'));
  assert.equal(rendered.queryByText('cobalt window'), null);
  assert.ok(rendered.getByRole('button', { name: 'Resume Offer' }));
});

test('losing focus leaves the non-sensitive device chooser open', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Add a device' }));
  const chooser = await rendered.findByRole('dialog');

  ui.fireEvent(window, new Event('blur'));

  assert.equal(rendered.getByRole('dialog'), chooser);
  assert.ok(rendered.getByRole('radiogroup', { name: 'What to add' }));
});

test('the recovery operations moved to a security key’s own page, each saying when it does not apply', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
    device: 'yubi:primary key',
  });

  const cardOps = rendered.getByRole('region', { name: 'Card operations' });
  // Every button the old "More…" menu and Connected row held, now grouped
  // under one heading, in the order Connected, Sync, Passphrase, Recovery —
  // the eleven moved operations — followed by the PIN row's unchanged link.
  await ui.waitFor(() => {
    assert.deepEqual(
      ui
        .within(cardOps)
        .getAllByRole('button')
        .map((button) => button.textContent),
      [
        'Provision…',
        'PIN Status',
        'Sync…',
        'Set…',
        'Change…',
        'Verify…',
        'Restore Management…',
        'Restore Signing Key…',
        'Resume Enrollment…',
        'Resume Rotation…',
        'Rotate…',
        'Settings › Server',
      ],
    );
  });
  const resume = ui
    .within(cardOps)
    .getByRole('button', { name: 'Resume Enrollment…' });
  // The fixture's "primary key" enrollment is already complete, so there is
  // nothing pending to resume.
  assert.equal(resume.hasAttribute('disabled'), true);
  assert.equal(
    resume.getAttribute('title'),
    'This enrollment is already complete.',
  );
  const rotate = ui.within(cardOps).getByRole('button', { name: 'Rotate…' });
  assert.equal(rotate.hasAttribute('disabled'), false);
  // Its card is connected (the fixture's one connected card matches its
  // serial), so PIN status is reachable too.
  const pinStatus = ui
    .within(cardOps)
    .getByRole('button', { name: 'PIN Status' });
  assert.equal(pinStatus.hasAttribute('disabled'), false);
});

test('a stopped account lists nothing and says why every action is off', async () => {
  const snapshot = await fixture();
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/index.ts',
  )) as typeof import('../src/model');
  const rendered = await renderDevices(applyLease(snapshot, 'lapsed'), {
    store: 'acct:work',
  });

  assert.ok(rendered.getByText('Account access is stopped'));
  assert.ok(rendered.getAllByText('Not listed while access is stopped').length);
  const add = rendered.getByRole('button', {
    name: 'Add a device',
  });
  assert.equal(add.hasAttribute('disabled'), true);
  assert.match(add.getAttribute('title') ?? '', /check-in.*expired/i);
  assert.equal(
    rendered.queryByRole('button', {
      name: 'Connect an Existing Account with a Recovery Phrase',
    }),
    null,
  );
});

test('a Mac with no account says so instead of listing an empty page', async () => {
  const snapshot = await fixture();
  const empty: AgentSnapshot = {
    ...snapshot,
    servers: [],
    accounts: [],
    stores: [],
    storeInventory: [],
    profileInventory: [],
    catalogProfiles: [],
    items: [],
    parties: [],
    federation: [],
    groupDetailFailures: [],
    notifications: [],
    observedExpiredLeases: [],
    plaintext: {},
  };
  const chosen: Location[] = [];
  const rendered = await renderDevices(empty, {
    onNavigate: (location) => chosen.push(location),
  });

  assert.ok(rendered.getByText('No account configured'));
  assert.ok(
    rendered.getByText(
      'Add or recover an account before configuring devices and security keys.',
    ),
  );
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Open Account' }));
  });
  assert.deepEqual(chosen.at(-1), { kind: 'settings', section: 'account' });
});

test('a Devices address written before the page was one lands on the one list', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
    section: 'keys',
  });

  // `section=macs` and `section=keys` both named a pane the page no longer
  // has; both now land on the one region the label reads.
  await ui.waitFor(() => {
    assert.equal(
      document.activeElement,
      rendered.getByRole('region', { name: 'Devices' }),
    );
  });
  // The Macs are on the same region, not behind a pane.
  assert.ok(rendered.getByText('Travel Mac'));
});

test('the hardware-key scene still opens its sheet on Devices', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
    scene: 'settings-enrol',
    // The scene's address names a section; the sheet keeps the keyboard.
    section: 'keys',
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(
    ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
    'Create an Account on a Hardware Key',
  );
  assert.ok(
    document.activeElement && dialog.contains(document.activeElement),
    'focus is contained within the dialog',
  );
  // Closing it hands the page back to the section the address named.
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Cancel' }),
    );
  });
  await ui.waitFor(() => {
    assert.equal(
      document.activeElement,
      rendered.getByRole('region', { name: 'Devices' }),
    );
  });
});

test('the recovery-phrase scene opens the one-time reveal', async () => {
  // `?state=settings-phrase` and `?state=settings&section=phrase` both land on
  // Devices, and the scene still opens the one-time sheet.
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
    scene: 'settings-phrase',
    // The scene's address also names a section; the sheet keeps the keyboard.
    section: 'macs',
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(
    ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
    'Save Recovery Phrase',
  );
  // The `section=` anchor must not pull focus out of the open sheet.
  await ui.waitFor(() => {
    assert.ok(
      document.activeElement && dialog.contains(document.activeElement),
      'focus is contained within the dialog',
    );
  });
});

/** Render the recovery-phrase scene, watching what it commits. */
async function recoveryPhrase(): Promise<{
  rendered: Awaited<ReturnType<typeof renderDevices>>;
  dialog: HTMLElement;
  committed: string[];
}> {
  const committed: string[] = [];
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
    scene: 'settings-phrase',
    section: 'macs',
    decorate: (bridge) => ({
      ...bridge,
      commitOwnerBackup: async (profile, account, alias, phrase) => {
        committed.push(phrase);
        return bridge.commitOwnerBackup(profile, account, alias, phrase);
      },
    }),
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.ok(
    dialog.querySelectorAll('.word').length > 1,
    'the sheet reveals the words',
  );
  return { rendered, dialog, committed };
}

test('Cancel discards a revealed recovery phrase rather than committing it', async () => {
  const { rendered, dialog, committed } = await recoveryPhrase();

  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Cancel' }),
    );
  });

  // `prepare_owner_backup` wrote nothing, so the phrase is simply gone: it is
  // off the screen and it was never committed.
  await ui.waitFor(() => assert.equal(rendered.queryByRole('dialog'), null));
  assert.equal(document.querySelectorAll('.word').length, 0);
  assert.deepEqual(committed, []);
});

test('Esc leaves the revealed recovery phrase the same way Cancel does', async () => {
  const { rendered, dialog, committed } = await recoveryPhrase();

  // Escape discards a revealed phrase, matching Cancel.
  await ui.act(async () => {
    ui.fireEvent.keyDown(dialog, { key: 'Escape' });
  });

  await ui.waitFor(() => assert.equal(rendered.queryByRole('dialog'), null));
  assert.equal(document.querySelectorAll('.word').length, 0);
  assert.deepEqual(committed, []);
});

test('the chooser leads into the recovery-phrase and provisioning sheets too', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });

  const paper = await choose(rendered, 'Create a new recovery phrase');
  assert.equal(
    ui.within(paper).getByRole('heading', { level: 2 }).textContent,
    'Create Recovery Phrase',
  );
  assert.ok(ui.within(paper).getByLabelText('Recovery phrase name'));
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(paper).getByRole('button', { name: 'Cancel' }),
    );
  });

  const provision = await choose(rendered, 'Connect a hardware key');
  assert.equal(
    ui.within(provision).getByRole('heading', { level: 2 }).textContent,
    'Connect a Hardware Key',
  );
  assert.ok(ui.within(provision).getByText('Hardware key 20993145'));
});

test('the chooser offers both pairing directions, so a phrase can be entered', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });

  const accepting = await choose(
    rendered,
    'Pair this device with another account',
  );
  assert.equal(
    ui.within(accepting).getByRole('heading', { level: 2 }).textContent,
    'Pair Another Device',
  );
  // The sheet opens on the accepting side: the phrase is typed, not revealed.
  assert.ok(ui.within(accepting).getByLabelText('Pairing phrase'));
  assert.ok(ui.within(accepting).getByRole('button', { name: 'Accept' }));
  assert.equal(
    ui.within(accepting).queryByRole('button', { name: 'Start' }),
    null,
  );
});

test('a revoke acts on the key whose details are open', async () => {
  const snapshot = await fixture();
  const twoKeys: AgentSnapshot = {
    ...snapshot,
    yubiAccounts: [
      ...snapshot.yubiAccounts,
      {
        alias: 'travel key',
        server: 'personal',
        serial: 20993147,
        state: 'complete' as const,
      },
    ],
  };
  const rendered = await renderDevices(twoKeys, {
    store: 'acct:personal',
    device: 'yubi:travel key',
  });

  // The details beside the list are the key's, and so is their Revoke….
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Revoke…' }));
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('alertdialog'));
  assert.equal(
    ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
    'Revoke travel key?',
  );
  assert.ok(ui.within(dialog).getByPlaceholderText('type travel key'));
});

test('an unfinished enrollment says so and cannot be revoked', async () => {
  // The work account's key was never finished, and the key list is the one
  // that account's server answered with — not the other account's.
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:work',
    device: 'yubi:work key',
  });

  const list = rendered.getByRole('region', { name: 'Devices' });
  assert.ok(ui.within(list).getByText('work key'));
  assert.equal(rendered.queryByText('primary key') === null, true);
  assert.ok(ui.within(list).getByText('Incomplete'));
  const revoke = rendered.getByRole('button', { name: 'Revoke…' });
  assert.equal(revoke.hasAttribute('disabled'), true);
  assert.equal(revoke.getAttribute('title'), 'This enrollment is not complete');

  // Switching back lists the other account's key and nothing of this one's.
  await rendered.showAccount('acct:personal');
  assert.ok(rendered.getByText('primary key'));
  assert.equal(rendered.queryByText('work key') === null, true);
  assert.ok(rendered.getByText('Enrolled'));
});

test('a pending enrollment whose card is absent offers only Resume enrollment', async () => {
  // "work key" is incomplete, and the one card the fixture reports connected
  // is matched to a different enrollment's serial — the card, from this
  // page, is absent.
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:work',
    device: 'yubi:work key',
  });

  const cardOps = rendered.getByRole('region', { name: 'Card operations' });
  const pinStatus = ui
    .within(cardOps)
    .getByRole('button', { name: 'PIN Status' });
  await ui.waitFor(() => {
    assert.equal(pinStatus.hasAttribute('disabled'), true);
    assert.equal(
      pinStatus.getAttribute('title'),
      'This key’s card is not connected.',
    );
  });
  const sync = ui.within(cardOps).getByRole('button', { name: 'Sync…' });
  assert.equal(sync.hasAttribute('disabled'), true);
  assert.equal(sync.getAttribute('title'), 'This enrollment is not complete.');
  // Resuming is the one operation a pending enrollment can use.
  const resume = ui
    .within(cardOps)
    .getByRole('button', { name: 'Resume Enrollment…' });
  assert.equal(resume.hasAttribute('disabled'), false);
  // Provisioning does not act on this enrollment specifically, so it stays
  // reachable regardless of this key's own state.
  const provision = ui
    .within(cardOps)
    .getByRole('button', { name: 'Provision…' });
  assert.equal(provision.hasAttribute('disabled'), false);
});

test('navigating to an unavailable account displays an error and lists available accounts', async () => {
  const chosen: Location[] = [];
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:nope',
    onNavigate: (location) => chosen.push(location),
  });

  assert.ok(rendered.getByText('Account no longer available'));
  // Not the empty-Mac notice: this Mac holds accounts, just not that one.
  assert.equal(rendered.queryByText('No account configured'), null);
  // The notice is the only list of the accounts this Mac does hold: the
  // switcher beside it would offer the same accounts a second time.
  assert.equal(
    rendered.queryByRole('group', { name: 'Accounts on this device' }),
    null,
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'personal · Personal server' }),
    );
  });
  assert.deepEqual(chosen.at(-1), { kind: 'devices', store: 'acct:personal' });
});

test('switching accounts from that notice leaves the lost key’s address behind', async () => {
  const chosen: Location[] = [];
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:nope',
    // The address was written on one key's own page, and that key belongs to
    // the account that is gone.
    device: `04${'d'.repeat(64)}`,
    onNavigate: (location) => chosen.push(location),
  });

  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'personal · Personal server' }),
    );
  });
  assert.deepEqual(chosen.at(-1), { kind: 'devices', store: 'acct:personal' });
});

test('a key on a card is revoked under its enrollment, not removed here', async () => {
  const card = `08${'5'.repeat(64)}`;
  const chosen: Location[] = [];
  const page = await renderDevices(await fixture(), {
    store: 'acct:personal',
    device: card,
    onNavigate: (location) => chosen.push(location),
  });

  assert.equal(
    page.getByRole('heading', { level: 1 }).textContent,
    'Pocket YubiKey',
  );
  const facts = page.getByRole('region', { name: 'This key' });
  assert.ok(ui.within(facts).getByText('Key on a card'));
  assert.ok(ui.within(facts).getByText(card));
  assert.ok(page.getByText('Revoked under its enrollment'));
  assert.equal(
    page.queryByRole('button', { name: 'Remove This Device…' }),
    null,
  );
  // The note is not a dead end, and it does not navigate to the page it is
  // already on: with no enrollment matching the card, it takes focus to the
  // Security keys rows of this list, leaving the selection alone.
  assert.equal(page.queryByRole('button', { name: 'Go to Devices' }), null);
  assert.ok(page.getByText(/listed under Security keys\./));
  const before = chosen.length;
  await ui.act(async () => {
    ui.fireEvent.click(
      page.getByRole('button', { name: 'Show Security Keys' }),
    );
  });
  assert.equal(chosen.length, before);
  assert.equal(
    document.activeElement,
    page.getByRole('option', { name: 'primary key' }),
  );
});

test('the device rows are one listbox the keyboard moves through', async () => {
  const snapshot = await fixture();
  const travel = deviceId(snapshot, 1);
  const moves: { location: Location; replace?: boolean }[] = [];
  const page = await renderDevices(snapshot, {
    store: 'acct:personal',
    device: travel,
    onNavigate: (location, options) =>
      moves.push({ location, replace: options?.replace }),
  });

  const listbox = page.getByRole('listbox', { name: 'Devices' });
  const options = ui.within(listbox).getAllByRole('option');
  // One Tab stop, on the selected row; nothing inside a row is tabbable.
  assert.deepEqual(
    options.map((option) => option.tabIndex),
    [-1, 0, -1, -1, -1],
  );
  const selected = page.getByRole('option', { name: 'Travel Mac' });
  assert.equal(selected.getAttribute('aria-selected'), 'true');
  selected.focus();

  // Selection follows the keyboard. A key is already open, so each move
  // replaces its address instead of adding a history entry per row.
  ui.fireEvent.keyDown(selected, { key: 'ArrowDown' });
  assert.equal(
    (moves.at(-1)?.location as { device?: string }).device,
    `08${'5'.repeat(64)}`,
  );
  assert.equal(moves.at(-1)?.replace, true);
  ui.fireEvent.keyDown(selected, { key: 'End' });
  assert.equal(
    (moves.at(-1)?.location as { device?: string }).device,
    options.at(-1)?.dataset.optionKey,
  );
  assert.equal(moves.length, 2);
  ui.fireEvent.keyDown(selected, { key: 'Home' });
  assert.equal(
    (moves.at(-1)?.location as { device?: string }).device,
    deviceId(snapshot, 0),
  );
});

test('the card this Mac is authenticated with is not told to remove itself', async () => {
  const card = `08${'5'.repeat(64)}`;
  const page = await renderDevices(await fixture(), {
    store: 'acct:personal',
    device: card,
    decorate: (bridge) => ({
      ...bridge,
      // The account is authenticated with the key on the card right now.
      listAccountDevices: async (store) =>
        (await bridge.listAccountDevices(store)).map((device) => ({
          ...device,
          current: device.id === card,
        })),
    }),
  });

  // Its kind decides this before its currency does: a card's key is revoked
  // under its enrollment either way, and no Mac is being asked to remove
  // itself.
  assert.ok(page.getByText('Authenticated on this device now'));
  assert.ok(page.getByText('Revoked under its enrollment'));
  assert.equal(page.queryByText('This device cannot be removed here'), null);
});

/** The mock bridge's id for a fixture device: `02…` is stored as `04…`. */
function deviceId(snapshot: AgentSnapshot, index: number): string {
  return snapshot.devices[index].id_hex.replace(/^02/, '04');
}

test('a key’s row opens its own page, with the full id and a way to copy it', async () => {
  const snapshot = await fixture();
  const travel = deviceId(snapshot, 1);
  const chosen: Location[] = [];
  const list = await renderDevices(snapshot, {
    store: 'acct:personal',
    onNavigate: (location) => chosen.push(location),
  });
  const row = list.getByText('Travel Mac').closest('.devrow');
  assert.ok(row);
  // The whole row is the control: a click on its name opens it too.
  ui.fireEvent.click(list.getByText('Travel Mac'));
  assert.equal(chosen.length, 1);
  await ui.act(async () => {
    ui.fireEvent.click(list.getByRole('option', { name: 'Travel Mac' }));
  });
  assert.equal(chosen.length, 2);
  assert.deepEqual(chosen.at(-1), {
    kind: 'devices',
    store: 'acct:personal',
    device: travel,
  });
  // The row carries no destructive action of its own.
  assert.equal(
    ui.within(row as HTMLElement).queryAllByRole('button').length,
    0,
  );
  ui.cleanup();

  const copied: string[] = [];
  const page = await renderDevices(snapshot, {
    store: 'acct:personal',
    device: travel,
    decorate: (bridge) => ({
      ...bridge,
      copyText: async (text: string) => {
        copied.push(text);
        return bridge.copyText(text);
      },
    }),
  });
  assert.equal(
    page.getByRole('heading', { level: 1 }).textContent,
    'Travel Mac',
  );
  // The row shows no id; the details carry the whole of it.
  const facts = page.getByRole('region', { name: 'This key' });
  assert.ok(ui.within(facts).getByText(travel));
  assert.ok(ui.within(facts).getByText('Computer'));
  assert.ok(ui.within(facts).getByText('Owner'));
  await ui.act(async () => {
    ui.fireEvent.click(page.getByRole('button', { name: 'Copy' }));
  });
  assert.deepEqual(copied, [travel]);

  await ui.act(async () => {
    ui.fireEvent.click(
      page.getByRole('button', { name: 'Remove This Device…' }),
    );
  });
  const dialog = await ui.waitFor(() => page.getByRole('alertdialog'));
  assert.equal(
    ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
    'Remove Travel Mac?',
  );
});

test('the local device detail page disables device removal', async () => {
  const snapshot = await fixture();
  const chosen: Location[] = [];
  const page = await renderDevices(snapshot, {
    store: 'acct:personal',
    device: deviceId(snapshot, 0),
    onNavigate: (location) => chosen.push(location),
  });

  const facts = page.getByRole('region', { name: 'This key' });
  assert.deepEqual(
    [...facts.querySelectorAll('.fr > .k')]
      .slice(0, 3)
      .map((label) => label.textContent),
    ['Account', 'Role on the account', 'Kind'],
  );
  assert.ok(page.getByText('Authenticated on this device now'));
  assert.ok(page.getByText('This device cannot be removed here'));
  assert.equal(
    page.queryByRole('button', { name: 'Remove This Device…' }),
    null,
  );
  await ui.act(async () => {
    ui.fireEvent.click(page.getByRole('button', { name: 'Open Settings' }));
  });
  assert.deepEqual(chosen.at(-1), { kind: 'settings', section: 'mac' });
  // The list stays beside the details; the location's parent is the list
  // at the same account.
  const { parentLocation } = await import('../src/location');
  assert.deepEqual(
    parentLocation({
      kind: 'devices',
      store: 'acct:personal',
      device: deviceId(snapshot, 0),
    }),
    { kind: 'devices', store: 'acct:personal' },
  );
});

test('a recovery phrase and an enrollment each carry their own page', async () => {
  const snapshot = await fixture();
  const backupId = `10${'4'.repeat(64)}`;
  const paper = await renderDevices(snapshot, {
    store: 'acct:personal',
    device: backupId,
  });
  const paperFacts = paper.getByRole('region', { name: 'This key' });
  assert.ok(ui.within(paperFacts).getByText('Recovery phrase'));
  assert.ok(ui.within(paperFacts).getByText(backupId));
  await ui.act(async () => {
    ui.fireEvent.click(paper.getByRole('button', { name: 'Revoke…' }));
  });
  const revokePaper = await ui.waitFor(() => paper.getByRole('alertdialog'));
  assert.equal(
    ui.within(revokePaper).getByRole('heading', { level: 2 }).textContent,
    'Revoke paper-backup?',
  );
  ui.cleanup();

  const key = await renderDevices(snapshot, {
    store: 'acct:personal',
    device: 'yubi:primary key',
  });
  // An enrollment has no id to copy, and the page says that rather than
  // showing an empty field.
  assert.equal(key.queryByRole('button', { name: 'Copy' }), null);
  const keyFacts = key.getByRole('region', { name: 'This key' });
  assert.ok(ui.within(keyFacts).getByText('Server'));
  assert.ok(ui.within(keyFacts).getByText('Personal server'));
  assert.ok(key.getByText('This enrollment applies across the server.'));
  assert.equal(key.queryByText('satoshi'), null);
  assert.ok(key.getByText(/Card serial .*; no device key recorded\./));
  assert.ok(key.getByRole('button', { name: 'Settings › Server' }));
  await ui.act(async () => {
    ui.fireEvent.click(key.getByRole('button', { name: 'Revoke…' }));
  });
  const revokeKey = await ui.waitFor(() => key.getByRole('alertdialog'));
  assert.equal(
    ui.within(revokeKey).getByRole('heading', { level: 2 }).textContent,
    'Revoke primary key?',
  );
});

test('an address naming a key this account no longer holds says so', async () => {
  const chosen: Location[] = [];
  const page = await renderDevices(await fixture(), {
    store: 'acct:personal',
    device: `04${'d'.repeat(64)}`,
    onNavigate: (location) => chosen.push(location),
  });

  assert.ok(page.getByText('This key is not on this account'));
  await ui.act(async () => {
    ui.fireEvent.click(page.getByRole('button', { name: 'Back to Devices' }));
  });
  assert.deepEqual(chosen.at(-1), { kind: 'devices', store: 'acct:personal' });
});

test('failed key-list reads do not report the key as absent', async () => {
  const failures: unknown[] = [];
  const page = await renderDevices(await fixture(), {
    store: 'acct:personal',
    device: `04${'d'.repeat(64)}`,
    onError: (error) => failures.push(error),
    decorate: (bridge) => ({
      ...bridge,
      listAccountDevices: async () => {
        throw {
          code: 'io',
          message: 'The account’s keys could not be read.',
          retryable: true,
          fatal: false,
          ambiguous: false,
        };
      },
    }),
  });

  await ui.waitFor(() =>
    assert.ok(page.getByText('Devices and keys could not be read')),
  );
  // The lists never answered, so the page does not assert the key is gone,
  // and the caption under it says the same thing about the same lists.
  assert.equal(page.queryByText('This key is not on this account'), null);
  assert.match(
    page.getByLabelText('device metadata freshness').textContent ?? '',
    /could not be loaded/,
  );
  assert.equal(failures.length, 1);
});

test('device metadata retry reloads device lists without refreshing the catalog', async () => {
  let reads = 0;
  let refreshes = 0;
  const snapshot = await fixture();
  const page = await renderDevices(snapshot, {
    store: 'acct:personal',
    onError: () => {},
    refresh: async () => {
      refreshes++;
      return snapshot;
    },
    decorate: (bridge) => ({
      ...bridge,
      listAccountDevices: async (store) => {
        reads++;
        if (reads === 1)
          throw {
            code: 'io',
            message: 'The account’s keys could not be read.',
            retryable: true,
            fatal: false,
            ambiguous: false,
          };
        return bridge.listAccountDevices(store);
      },
    }),
  });
  await ui.waitFor(() =>
    assert.match(
      page.getByLabelText('device metadata freshness').textContent ?? '',
      /could not be loaded/,
    ),
  );
  assert.equal(page.queryByText('This device'), null);
  await ui.act(async () => {
    ui.fireEvent.click(page.getByRole('button', { name: 'Retry' }));
  });
  // The second read answers, so this Mac's own row is listed and the caption
  // has nothing more to say. No catalog refresh was asked for on its behalf.
  await ui.waitFor(() => assert.ok(page.getByText('This device')));
  assert.equal(reads, 2);
  assert.equal(refreshes, 0);
  assert.equal(page.queryByLabelText('device metadata freshness'), null);
});

test('pairing is two numbered steps, and a resumed offer says the agent holds it', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });
  const openPairing = async (): Promise<HTMLElement> => {
    await ui.act(async () => {
      ui.fireEvent.click(
        rendered.getByRole('button', { name: 'Add a device' }),
      );
    });
    await ui.act(async () => {
      ui.fireEvent.click(rendered.getByRole('button', { name: 'Continue' }));
    });
    return ui.waitFor(() => rendered.getByRole('dialog'));
  };

  const pairing = await openPairing();
  // Two steps, each written for the device it is done on.
  assert.ok(ui.within(pairing).getByText('On this device'));
  assert.ok(ui.within(pairing).getByText('On the other device'));
  // The chooser named the direction, so no control here quietly changes it.
  assert.equal(ui.within(pairing).queryByText('Get a phrase'), null);
  // Step 1 has not produced a phrase yet, so step 2 has nothing to do and
  // what it ends with cannot run.
  assert.ok(ui.within(pairing).getByText('Get a pairing phrase.'));
  assert.equal(
    ui
      .within(pairing)
      .getByRole('button', { name: 'Finish' })
      .hasAttribute('disabled'),
    true,
  );

  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(pairing).getByRole('button', { name: 'Start' }),
    );
  });
  assert.ok(ui.within(pairing).getByText('cobalt window'));
  assert.equal(
    ui
      .within(pairing)
      .getByRole('button', { name: 'Finish' })
      .hasAttribute('disabled'),
    false,
  );

  // Closing the sheet does not drop the offer the agent is holding, and
  // resuming shows the same phrase with a band saying so.
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(pairing).getByRole('button', { name: 'Close' }),
    );
  });
  const resumed = await openPairing();
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(resumed).getByRole('button', { name: 'Resume Offer' }),
    );
  });
  assert.ok(
    ui.within(resumed).getByText('A pairing is waiting on this device'),
  );
  assert.ok(ui.within(resumed).getByText('cobalt window'));
});

test('switching accounts drops the last account’s lists and closes an open sheet', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });
  assert.ok(rendered.getByText('Travel Mac'));
  assert.ok(rendered.getByText('paper-backup'));

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Add a device' }));
  });
  assert.ok(await ui.waitFor(() => rendered.getByRole('dialog')));

  await rendered.showAccount('acct:work');

  assert.equal(rendered.queryByRole('dialog'), null);
  assert.equal(rendered.queryByText('Travel Mac'), null);
  assert.equal(rendered.queryByText('paper-backup'), null);
  // The work account's own device and key are listed in their place; it
  // holds no recovery phrase, so none is silently carried over from personal.
  assert.ok(await rendered.findByText('work key'));
});

test('a revealed recovery phrase stays open when the app loses focus', async () => {
  const { rendered, dialog, committed } = await recoveryPhrase();
  const words = dialog.querySelector('.words')?.textContent;
  ui.fireEvent(window, new Event('blur'));
  assert.equal(rendered.getByRole('dialog'), dialog);
  assert.equal(dialog.querySelector('.words')?.textContent, words);

  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    value: 'hidden',
  });
  ui.fireEvent(document, new Event('visibilitychange'));
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    value: 'visible',
  });
  assert.equal(rendered.getByRole('dialog'), dialog);
  assert.equal(dialog.querySelector('.words')?.textContent, words);
  assert.deepEqual(committed, []);
});

test('PIN Status on a key’s own page acts on that key, not on card or list order', async () => {
  const calls: unknown[] = [];
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
    // "second" is neither the first card nor the first enrollment either
    // list answers with; its own page must still act on it specifically.
    device: 'yubi:second',
    decorate: (base) => ({
      ...base,
      listYubiCards: async () => [{ serial: 111 }, { serial: 222 }],
      listYubiAccounts: async () => [
        { alias: 'second', state: 'complete', cardSerial: 222 },
        { alias: 'first', state: 'complete', cardSerial: 111 },
      ],
      runYubi: async (command) => {
        calls.push(command);
        return { remaining: 3, blocked: false };
      },
    }),
  });
  // The hardware scan is in the bar's More menu.
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'More' }));
  });
  ui.fireEvent.click(
    await rendered.findByRole('menuitem', { name: 'Refresh Hardware Keys' }),
  );
  const pinStatus = await rendered.findByRole('button', {
    name: 'PIN Status',
  });
  await ui.waitFor(() =>
    assert.equal(pinStatus.hasAttribute('disabled'), false),
  );
  ui.fireEvent.click(pinStatus);
  const dialog = rendered.getByRole('dialog');
  assert.ok(ui.within(dialog).getAllByText('second').length);
  ui.fireEvent.click(
    ui.within(dialog).getByRole('button', { name: 'Continue' }),
  );
  await ui.waitFor(() => assert.equal(calls.length, 1));
  assert.deepEqual(calls[0], {
    command: 'yubi_pin_status',
    args: { profile: 'personal', alias: 'second' },
  });
});

test('a card-backed device opens its enrollment by the native key id', async () => {
  const id = '0802' + 'ab'.repeat(32);
  const locations: Location[] = [];
  await renderDevices(await fixture(), {
    store: 'acct:personal',
    device: id,
    onNavigate: (at) => locations.push(at),
    decorate: (base) => ({
      ...base,
      listAccountDevices: async () => [
        { id, name: 'Travel device', role: 'owner', current: false },
      ],
      listYubiAccounts: async () => [
        { alias: 'travel', state: 'complete', deviceId: id, cardSerial: 111 },
      ],
    }),
  });
  // The details' note, not the list's row of the same name.
  const note = await ui.waitFor(() => {
    const found = document.querySelector<HTMLElement>(
      '.devices-detail .notice',
    );
    assert.ok(found);
    return found;
  });
  ui.fireEvent.click(
    ui.within(note).getByRole('button', { name: 'Open travel' }),
  );
  assert.deepEqual(locations.at(-1), {
    kind: 'devices',
    section: 'keys',
    store: 'acct:personal',
    device: 'yubi:travel',
  });
});

test('Devices reads only the current account and follows sidebar account changes', async () => {
  const reads: StoreRef[] = [];
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
    decorate: (base) => ({
      ...base,
      listAccountDevices: async (store) => {
        reads.push(store);
        return [
          {
            id: '04' + 'ab'.repeat(32),
            name: store === 'acct:personal' ? 'Personal Mac' : 'Work Mac',
            role: 'owner',
            current: true,
          },
        ];
      },
      listBackupEnrollments: async () => [],
      listYubiAccounts: async () => [],
      listYubiCards: async () => [],
    }),
  });
  // One computer, no recovery phrases and no enrollments: the list holds exactly
  // the one row, with no empty-category filler for the two empty lists.
  assert.ok(await rendered.findByText('Personal Mac'));
  assert.equal(rendered.queryByText('Work Mac'), null);
  assert.equal(
    rendered.queryByRole('group', { name: 'Accounts on this device' }),
    null,
  );
  // Exactly one row: no empty-category filler for the recovery phrases and
  // enrollments that came back empty.
  assert.equal(rendered.getAllByRole('option').length, 1);
  assert.equal(
    rendered.queryByRole('button', {
      name: 'Connect an Existing Account with a Recovery Phrase',
    }),
    null,
  );
  await rendered.showAccount('acct:work');
  assert.ok(await rendered.findByText('Work Mac'));
  assert.equal(rendered.queryByText('Personal Mac'), null);
  assert.deepEqual(reads, ['acct:personal', 'acct:work']);
});

test('device removal refreshes native device records before submitting the write', async () => {
  const snapshot = await fixture();
  const target = deviceId(snapshot, 1);
  const calls: string[] = [];
  const page = await renderDevices(snapshot, {
    store: 'acct:personal',
    device: target,
    decorate: (bridge) => ({
      ...bridge,
      listAccountDevices: async (store) => {
        calls.push('read');
        return bridge.listAccountDevices(store);
      },
      removeAccountDevice: async (store, device) => {
        calls.push('remove');
        return bridge.removeAccountDevice(store, device);
      },
    }),
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      page.getByRole('button', { name: 'Remove This Device…' }),
    );
  });
  const input = document.querySelector<HTMLInputElement>('.sheet input');
  assert.ok(input);
  ui.fireEvent.change(input, {
    target: { value: input.placeholder.replace(/^type /, '') },
  });
  calls.length = 0;
  await ui.act(async () => {
    ui.fireEvent.click(page.getByRole('button', { name: 'Remove Device' }));
  });
  // The fresh authorization read precedes the write; subscribed display
  // metadata is invalidated and refreshed only after successful completion.
  assert.deepEqual(calls, ['read', 'remove', 'read']);
});

test('an account with no recovery phrase gets its own band without publishing shell alerts', async () => {
  const { deviceAlertRegistry } = (await vite.ssrLoadModule(
    '/src/screens/device-alert.ts',
  )) as typeof import('../src/screens/device-alert');
  const snapshot = await fixture();
  const bridgeRef: { current: Bridge | null } = { current: null };
  const page = await renderDevices(snapshot, {
    store: 'acct:work',
    decorate: (bridge) => {
      bridgeRef.current = bridge;
      return bridge;
    },
  });
  assert.ok(bridgeRef.current);
  await ui.waitFor(() => {
    assert.ok(page.getByText('No recovery key'));
  });
  assert.equal(
    deviceAlertRegistry(bridgeRef.current)
      .getSnapshot()
      .recoveryPhrases.get('acct:work'),
    undefined,
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      page.getByRole('button', { name: 'Create a Recovery Phrase…' }),
    );
  });
  assert.ok(document.querySelector('.sheet'));
  assert.ok(page.getByRole('heading', { name: 'Create Recovery Phrase' }));
});

test('an account with a recovery phrase draws no band without publishing shell alerts', async () => {
  const { deviceAlertRegistry } = (await vite.ssrLoadModule(
    '/src/screens/device-alert.ts',
  )) as typeof import('../src/screens/device-alert');
  const snapshot = await fixture();
  const bridgeRef: { current: Bridge | null } = { current: null };
  const page = await renderDevices(snapshot, {
    store: 'acct:personal',
    decorate: (bridge) => {
      bridgeRef.current = bridge;
      return bridge;
    },
  });
  assert.ok(bridgeRef.current);
  await ui.waitFor(() => assert.ok(page.getByText('paper-backup')));
  assert.equal(
    deviceAlertRegistry(bridgeRef.current)
      .getSnapshot()
      .recoveryPhrases.get('acct:personal'),
    undefined,
  );
  assert.equal(page.queryByText('No recovery key'), null);
});
