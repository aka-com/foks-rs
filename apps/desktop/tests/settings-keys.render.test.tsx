/**
 * The Devices tab: Macs, paper keys and security keys in one list on one
 * page, the chooser that adds one, the recovery operations grouped on a
 * security key's own page, and the scenes that still open a sheet on
 * arrival.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge } from '../src/bridge';
import type { Location } from '../src/location';
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
  /** One key's own page: the id of a Mac or paper key, or `yubi:<alias>`. */
  device?: string;
  onNavigate?: (location: Location) => void;
  /** Wraps the mock bridge, for a test that watches one command. */
  decorate?: (bridge: Bridge) => Bridge;
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
          onRefreshSnapshot: async () => snapshot,
          onError: (error: unknown) => {
            throw error;
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
  const showAccount = async (at: StoreRef): Promise<void> => {
    await ui.act(async () => {
      rendered.rerender(page(at));
      await Promise.resolve();
    });
  };
  return Object.assign(rendered, { showAccount });
}

test('one page lists every device, paper key and security key together', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });

  // One region, one heading — no more "Computers and security keys", "Paper
  // keys" or "Security keys" sections.
  assert.ok(rendered.getByRole('region', { name: 'Devices' }));
  assert.equal(rendered.queryByText('Computers and security keys'), null);
  assert.equal(rendered.queryByText('Paper keys'), null);
  assert.equal(rendered.queryByText('Security keys'), null);
  assert.equal(
    rendered.queryByRole('group', { name: 'Accounts on this device' }),
    null,
  );

  // This Mac says so, with the same "Current" chip every current row reads,
  // and cannot remove itself; the other Mac can be removed, and the key on a
  // card is neither — it is revoked under its enrollment.
  assert.ok(rendered.getByText('Current'));
  assert.equal(rendered.getAllByRole('button', { name: 'Remove…' }).length, 1);
  assert.ok(rendered.getByText('Pocket YubiKey'));
  // Its caption names its hardware type, and its chip uses the user-facing
  // security-key label.
  assert.equal(rendered.getAllByText('Security key').length, 2);
  assert.ok(rendered.getByText('Key on a card'));
  assert.equal(
    rendered.queryByRole('button', { name: 'Open Pocket YubiKey' })?.tagName,
    'BUTTON',
  );
  assert.ok(rendered.getByText('MacBook Pro'));
  assert.ok(rendered.getByText('Travel Mac'));
  // Every row's caption names its type, not its role — the role moved to the
  // key's own page.
  assert.equal(rendered.getAllByText('Computer').length, 2);
  assert.ok(rendered.getByText('paper-backup'));
  assert.ok(rendered.getByText('Paper key'));
  assert.ok(rendered.getByText('primary key'));
  // The list is sorted by type: computers (and any key on a card among
  // them), then paper keys, then security key enrollments.
  const names = rendered
    .getAllByRole('button', { name: /^Open / })
    .map((button) => button.getAttribute('aria-label'));
  assert.deepEqual(names, [
    'Open MacBook Pro',
    'Open Travel Mac',
    'Open Pocket YubiKey',
    'Open paper-backup',
    'Open primary key',
  ]);
  // There is no sub-navigation left on this page.
  assert.equal(rendered.queryByText('Recovery devices'), null);
  // Neither the section-level "Add a paper key…" button nor the "More…" menu
  // survive: "Add a device" is the only add control left.
  assert.equal(
    rendered.queryByRole('button', { name: 'Add a paper key…' }),
    null,
  );
  assert.equal(rendered.queryByRole('button', { name: 'More…' }), null);
  assert.ok(rendered.getByRole('button', { name: 'Add a device' }));
  // The recovery action is still reachable, as a quiet link below the list.
  assert.ok(
    rendered.getByRole('button', {
      name: 'Recover an account with a paper key…',
    }),
  );
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
      'Create a new recovery paper key',
      'Connect a new YubiKey',
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
    'Pair another device',
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
        'PIN status',
        'Sync…',
        'Set…',
        'Change…',
        'Verify…',
        'Restore management…',
        'Restore signing key…',
        'Resume enrollment…',
        'Resume rotation…',
        'Rotate…',
        'Settings › Server',
      ],
    );
  });
  const resume = ui
    .within(cardOps)
    .getByRole('button', { name: 'Resume enrollment…' });
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
    .getByRole('button', { name: 'PIN status' });
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
  assert.match(add.getAttribute('title') ?? '', /Check-in expired/);
  const recover = rendered.getByRole('button', {
    name: 'Recover an account with a paper key…',
  });
  assert.equal(recover.hasAttribute('disabled'), true);
  assert.match(recover.getAttribute('title') ?? '', /Check-in expired/);
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
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Open Accounts' }));
  });
  assert.deepEqual(chosen.at(-1), { kind: 'people' });
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

test('the YubiKey scene still opens its sheet on Devices', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
    scene: 'settings-enrol',
    // The scene's address names a section; the sheet keeps the keyboard.
    section: 'keys',
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(
    ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
    'Create a YubiKey account',
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

test('the paper-key scene opens the one-time reveal', async () => {
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
    'Save paper key',
  );
  // The `section=` anchor must not pull focus out of the open sheet.
  await ui.waitFor(() => {
    assert.ok(
      document.activeElement && dialog.contains(document.activeElement),
      'focus is contained within the dialog',
    );
  });
});

/** Render the paper-key scene, watching what it commits. */
async function paperKey(): Promise<{
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

test('Cancel discards a revealed paper key rather than committing it', async () => {
  const { rendered, dialog, committed } = await paperKey();

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

test('Esc leaves the revealed paper key the same way Cancel does', async () => {
  const { rendered, dialog, committed } = await paperKey();

  // Escape discards a revealed phrase, matching Cancel.
  await ui.act(async () => {
    ui.fireEvent.keyDown(dialog, { key: 'Escape' });
  });

  await ui.waitFor(() => assert.equal(rendered.queryByRole('dialog'), null));
  assert.equal(document.querySelectorAll('.word').length, 0);
  assert.deepEqual(committed, []);
});

test('the chooser leads into the paper-key and provisioning sheets too', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });

  const paper = await choose(rendered, 'Create a new recovery paper key');
  assert.equal(
    ui.within(paper).getByRole('heading', { level: 2 }).textContent,
    'Create paper key',
  );
  assert.ok(ui.within(paper).getByLabelText('Paper key name'));
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(paper).getByRole('button', { name: 'Cancel' }),
    );
  });

  const provision = await choose(rendered, 'Connect a new YubiKey');
  assert.equal(
    ui.within(provision).getByRole('heading', { level: 2 }).textContent,
    'Connect a YubiKey',
  );
  assert.ok(ui.within(provision).getByText('YubiKey 20993145'));
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
    'Pair another device',
  );
  // The sheet opens on the accepting side: the phrase is typed, not revealed.
  assert.ok(ui.within(accepting).getByLabelText('Pairing phrase'));
  assert.ok(ui.within(accepting).getByRole('button', { name: 'Accept' }));
  assert.equal(
    ui.within(accepting).queryByRole('button', { name: 'Start' }),
    null,
  );
});

test('a revoke acts on the key whose row was pressed', async () => {
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
  const rendered = await renderDevices(twoKeys, { store: 'acct:personal' });

  // The list also carries the paper key's own Revoke…, so the row is found
  // by name rather than by counting every Revoke… button on the page.
  const row = rendered.getByText('travel key').closest('.fr');
  assert.ok(row instanceof HTMLElement);
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(row).getByRole('button', { name: 'Revoke…' }));
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
  const rendered = await renderDevices(await fixture(), { store: 'acct:work' });

  assert.ok(rendered.getByText('work key'));
  assert.equal(rendered.queryByText('primary key'), null);
  assert.ok(rendered.getByText('Incomplete'));
  const row = rendered.getByText('work key').closest('.fr');
  assert.ok(row instanceof HTMLElement);
  const revoke = ui.within(row).getByRole('button', { name: 'Revoke…' });
  assert.equal(revoke.hasAttribute('disabled'), true);
  assert.equal(revoke.getAttribute('title'), 'This enrollment is not complete');

  // Switching back lists the other account's key and nothing of this one's.
  await rendered.showAccount('acct:personal');
  assert.ok(rendered.getByText('primary key'));
  assert.equal(rendered.queryByText('work key'), null);
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
    .getByRole('button', { name: 'PIN status' });
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
    .getByRole('button', { name: 'Resume enrollment…' });
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
  assert.ok(page.getByText('Key on a card'));
  assert.ok(page.getByText(card));
  assert.ok(page.getByText('Revoked under its enrollment'));
  assert.equal(
    page.queryByRole('button', { name: 'Remove this device…' }),
    null,
  );
  // The row is not a dead end: it leads back to the list that revokes the key.
  await ui.act(async () => {
    ui.fireEvent.click(page.getByRole('button', { name: 'Go to Devices' }));
  });
  assert.deepEqual(chosen.at(-1), {
    kind: 'devices',
    section: 'keys',
    store: 'acct:personal',
  });
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
  await ui.act(async () => {
    ui.fireEvent.click(list.getByRole('button', { name: 'Open Travel Mac' }));
  });
  assert.deepEqual(chosen.at(-1), {
    kind: 'devices',
    store: 'acct:personal',
    device: travel,
  });
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
  // The row shortened the id; the page carries the whole of it.
  assert.ok(page.getByText(travel));
  assert.ok(page.getByText('Computer'));
  assert.ok(page.getByText('Owner'));
  await ui.act(async () => {
    ui.fireEvent.click(page.getByRole('button', { name: 'Copy' }));
  });
  assert.deepEqual(copied, [travel]);

  await ui.act(async () => {
    ui.fireEvent.click(
      page.getByRole('button', { name: 'Remove this device…' }),
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

  assert.ok(page.getByText('Authenticated on this device now'));
  assert.ok(page.getByText('This device cannot be removed here'));
  assert.equal(
    page.queryByRole('button', { name: 'Remove this device…' }),
    null,
  );
  await ui.act(async () => {
    ui.fireEvent.click(page.getByRole('button', { name: 'Open Settings' }));
  });
  assert.deepEqual(chosen.at(-1), { kind: 'settings', section: 'about' });
  // The way back to the list is the topbar's chevron, which the shell draws
  // from the location alone.
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

test('a paper key and an enrollment each carry their own page', async () => {
  const snapshot = await fixture();
  const backupId = `10${'4'.repeat(64)}`;
  const paper = await renderDevices(snapshot, {
    store: 'acct:personal',
    device: backupId,
  });
  assert.ok(paper.getByText('Paper key'));
  assert.ok(paper.getByText(backupId));
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
  assert.ok(key.getByText('Server'));
  assert.ok(key.getByText('Personal server'));
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
      ui.within(resumed).getByRole('button', { name: 'Resume offer' }),
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
  // holds no paper key, so none is silently carried over from personal.
  assert.ok(await rendered.findByText('work key'));
});

test('a paper key hidden by blur can be recovered once', async () => {
  const { rendered, dialog, committed } = await paperKey();
  const words = dialog.querySelector('.words')?.textContent;
  ui.fireEvent(window, new Event('blur'));
  assert.equal(rendered.queryByRole('dialog'), null);
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Show the paper key again' }),
  );
  const resumed = rendered.getByRole('dialog');
  assert.equal(resumed.querySelector('.words')?.textContent, words);
  ui.fireEvent(window, new Event('blur'));
  assert.equal(rendered.queryByRole('dialog'), null);
  assert.equal(
    rendered.queryByRole('button', { name: 'Show the paper key again' }),
    null,
  );
  assert.deepEqual(committed, []);
});

test('PIN status on a key’s own page acts on that key, not on card or list order', async () => {
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
  const pinStatus = await rendered.findByRole('button', {
    name: 'PIN status',
  });
  assert.equal(pinStatus.hasAttribute('disabled'), false);
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
  const rendered = await renderDevices(await fixture(), {
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
  ui.fireEvent.click(
    await rendered.findByRole('button', { name: 'Open travel' }),
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
  // One computer, no paper keys and no enrollments: the list holds exactly
  // the one row, with no empty-category filler for the two lists that came
  // back empty, and the recovery link stays reachable below it.
  assert.ok(await rendered.findByText('Personal Mac'));
  assert.equal(rendered.queryByText('Work Mac'), null);
  assert.equal(
    rendered.queryByRole('group', { name: 'Accounts on this device' }),
    null,
  );
  // Exactly one row: no empty-category filler for the paper keys and
  // enrollments that came back empty.
  assert.equal(rendered.getAllByRole('button', { name: /^Open / }).length, 1);
  assert.ok(
    rendered.getByRole('button', {
      name: 'Recover an account with a paper key…',
    }),
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
      page.getByRole('button', { name: 'Remove this device…' }),
    );
  });
  const input = document.querySelector<HTMLInputElement>('.sheet input');
  assert.ok(input);
  ui.fireEvent.change(input, {
    target: { value: input.placeholder.replace(/^type /, '') },
  });
  calls.length = 0;
  await ui.act(async () => {
    ui.fireEvent.click(page.getByRole('button', { name: 'Remove device' }));
  });
  // The fresh authorization read precedes the write; subscribed display
  // metadata is invalidated and refreshed only after successful completion.
  assert.deepEqual(calls, ['read', 'remove', 'read']);
});
