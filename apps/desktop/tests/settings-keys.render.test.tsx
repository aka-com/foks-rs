/**
 * The Devices tab: Macs, paper keys and security keys on one page, the chooser
 * that adds one, the recovery operations behind the section's menu, and the
 * scenes that still open a sheet on arrival.
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

test('one page lists the Macs, the paper keys and the security keys', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });

  assert.ok(rendered.getByText('Macs and device keys'));
  assert.ok(rendered.getByText('Paper keys'));
  assert.ok(rendered.getByText('Security key enrollments'));
  assert.equal(
    rendered.queryByText('Only paper keys stored on this Mac are listed.'),
    null,
  );
  assert.equal(rendered.queryByText(/Enrollments are listed for/), null);
  assert.equal(
    rendered.queryByRole('group', { name: 'Accounts on this Mac' }),
    null,
  );

  // This Mac says so and cannot remove itself; the other Mac can be removed,
  // and the key on a card is neither — it is revoked under its enrollment.
  assert.ok(rendered.getByText('This device'));
  assert.equal(rendered.getAllByRole('button', { name: 'Remove…' }).length, 1);
  const macs = rendered.getByRole('region', { name: 'Macs and device keys' });
  assert.ok(ui.within(macs).getByText('Pocket YubiKey'));
  assert.ok(ui.within(macs).getByText('Key on a card'));
  assert.equal(
    ui.within(macs).queryByRole('button', { name: 'Open Pocket YubiKey' })
      ?.tagName,
    'BUTTON',
  );
  assert.ok(rendered.getByText('MacBook Pro'));
  assert.ok(rendered.getByText('Travel Mac'));
  assert.ok(rendered.getAllByText('Owner').length);
  assert.ok(rendered.getByText('paper-backup'));
  assert.ok(rendered.getByText('primary key'));
  assert.ok(rendered.getByText('YubiKey 20993145'));
  // There is no sub-navigation left on this page.
  assert.equal(rendered.queryByText('Recovery devices'), null);
});

/** Open the chooser and continue on the card with this title. */
async function choose(
  rendered: Awaited<ReturnType<typeof renderDevices>>,
  title: string,
): Promise<HTMLElement> {
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Add a device or paper key' }),
    );
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
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Add a device or paper key' }),
    );
  });
  const chooser = await ui.waitFor(() => rendered.getByRole('dialog'));
  const choices = ui.within(chooser).getAllByRole('radio');
  assert.deepEqual(
    choices.map((choice) => choice.querySelector('b')?.textContent),
    ['Pair another Mac', 'Enter a pairing phrase', 'Paper key', 'Security key'],
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(chooser).getByRole('button', { name: 'Continue' }),
    );
  });
  const pairing = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(
    ui.within(pairing).getByRole('heading', { level: 2 }).textContent,
    'Set up another Mac',
  );
  assert.ok(ui.within(pairing).getByText('On the other Mac'));
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
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Add a device or paper key' }),
    );
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

test('the recovery operations stay reachable, each saying when it does not apply', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'More…' }));
  });
  const menu = await ui.waitFor(() =>
    rendered.getByRole('menu', { name: 'Security key operations' }),
  );
  const items = ui.within(menu).getAllByRole('menuitem');
  assert.equal(items.length, 10);
  const resume = items.find((item) =>
    item.textContent?.startsWith('Resume enrollment'),
  );
  assert.ok(resume);
  // The fixture holds one complete enrollment and no pending one.
  assert.equal(resume.getAttribute('aria-disabled'), 'true');
  assert.equal(resume.getAttribute('title'), 'No pending enrollment found');
  const rotate = items.find((item) =>
    item.textContent?.startsWith('Rotate the management key'),
  );
  assert.ok(rotate);
  assert.equal(rotate.getAttribute('aria-disabled'), null);
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
    name: 'Add a device or paper key',
  });
  assert.equal(add.hasAttribute('disabled'), true);
  assert.match(add.getAttribute('title') ?? '', /Check-in expired/);
  assert.equal(
    rendered
      .getByRole('button', { name: 'Add a paper key…' })
      .hasAttribute('disabled'),
    true,
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
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Open Accounts' }));
  });
  assert.deepEqual(chosen.at(-1), { kind: 'people' });
});

test('a Devices address written before the page was one lands on its section', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
    section: 'keys',
  });

  // The section is a labelled region, named by the label the reader sees.
  await ui.waitFor(() => {
    assert.equal(
      document.activeElement,
      rendered.getByRole('region', { name: 'Security key enrollments' }),
    );
  });
  // The Macs are still on the same page, not behind a pane.
  assert.ok(rendered.getByRole('region', { name: 'Macs and device keys' }));
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
      rendered.getByRole('region', { name: 'Security key enrollments' }),
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

  const paper = await choose(rendered, 'Paper key');
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

  const provision = await choose(rendered, 'Security key');
  assert.equal(
    ui.within(provision).getByRole('heading', { level: 2 }).textContent,
    'Provision a YubiKey device',
  );
  assert.ok(ui.within(provision).getByText('YubiKey 20993145'));
});

test('the chooser offers both pairing directions, so a phrase can be entered', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });

  const accepting = await choose(rendered, 'Enter a pairing phrase');
  assert.equal(
    ui.within(accepting).getByRole('heading', { level: 2 }).textContent,
    'Set up another Mac',
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

  const keys = rendered.getByRole('region', {
    name: 'Security key enrollments',
  });
  const revokes = ui.within(keys).getAllByRole('button', { name: 'Revoke…' });
  assert.equal(revokes.length, 2);
  await ui.act(async () => {
    ui.fireEvent.click(revokes[1]);
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

  const keys = rendered.getByRole('region', {
    name: 'Security key enrollments',
  });
  assert.ok(ui.within(keys).getByText('work key'));
  assert.equal(ui.within(keys).queryByText('primary key'), null);
  assert.ok(ui.within(keys).getByText('Incomplete'));
  const revoke = ui.within(keys).getByRole('button', { name: 'Revoke…' });
  assert.equal(revoke.hasAttribute('disabled'), true);
  assert.equal(revoke.getAttribute('title'), 'This enrollment is not complete');

  // Switching back lists the other account's key and nothing of this one's.
  await rendered.showAccount('acct:personal');
  const listed = rendered.getByRole('region', {
    name: 'Security key enrollments',
  });
  assert.ok(ui.within(listed).getByText('primary key'));
  assert.equal(ui.within(listed).queryByText('work key'), null);
  assert.ok(ui.within(listed).getByText('Enrolled'));
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
    rendered.queryByRole('group', { name: 'Accounts on this Mac' }),
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
  // The row is not a dead end: it leads to the section that revokes the key.
  await ui.act(async () => {
    ui.fireEvent.click(
      page.getByRole('button', { name: 'Go to Security key enrollments' }),
    );
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
  assert.ok(page.getByText('Authenticated on this Mac now'));
  assert.ok(page.getByText('Revoked under its enrollment'));
  assert.equal(page.queryByText('This Mac cannot remove itself'), null);
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

  assert.ok(page.getByText('Authenticated on this Mac now'));
  assert.ok(page.getByText('This Mac cannot remove itself'));
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
        rendered.getByRole('button', { name: 'Add a device or paper key' }),
      );
    });
    await ui.act(async () => {
      ui.fireEvent.click(rendered.getByRole('button', { name: 'Continue' }));
    });
    return ui.waitFor(() => rendered.getByRole('dialog'));
  };

  const pairing = await openPairing();
  // Two steps, each written for the Mac it is done on.
  assert.ok(ui.within(pairing).getByText('On this Mac'));
  assert.ok(ui.within(pairing).getByText('On the other Mac'));
  // The chooser named the direction, so no control here quietly changes it.
  assert.equal(ui.within(pairing).queryByText('Get a phrase'), null);
  // Step 1 has not produced a phrase yet, so step 2 has nothing to do and
  // what it ends with cannot run.
  assert.ok(ui.within(pairing).getByText(/^Get a pairing phrase for/));
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
  assert.ok(ui.within(resumed).getByText('A pairing is waiting on this Mac'));
  assert.ok(ui.within(resumed).getByText('cobalt window'));
});

test('switching accounts drops the last account’s lists and closes an open sheet', async () => {
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
  });
  assert.ok(rendered.getByText('Travel Mac'));
  assert.ok(rendered.getByText('paper-backup'));

  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Add a device or paper key' }),
    );
  });
  assert.ok(await ui.waitFor(() => rendered.getByRole('dialog')));

  await rendered.showAccount('acct:work');

  assert.equal(rendered.queryByRole('dialog'), null);
  assert.equal(rendered.queryByText('Travel Mac'), null);
  assert.equal(rendered.queryByText('paper-backup'), null);
  assert.ok(
    rendered.getByText('No paper keys stored on this Mac for this account.'),
  );
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

test('PIN status selects the enrollment on the requested card rather than list order', async () => {
  const calls: unknown[] = [];
  const rendered = await renderDevices(await fixture(), {
    store: 'acct:personal',
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
  const buttons = await rendered.findAllByRole('button', {
    name: 'PIN status',
  });
  ui.fireEvent.click(buttons[0]);
  const dialog = rendered.getByRole('dialog');
  assert.ok(ui.within(dialog).getAllByText('first').length);
  ui.fireEvent.click(
    ui.within(dialog).getByRole('button', { name: 'Continue' }),
  );
  await ui.waitFor(() => assert.equal(calls.length, 1));
  assert.deepEqual(calls[0], {
    command: 'yubi_pin_status',
    args: { profile: 'personal', alias: 'first' },
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
  assert.ok(await rendered.findByText('Personal Mac'));
  assert.equal(rendered.queryByText('Work Mac'), null);
  assert.equal(
    rendered.queryByRole('group', { name: 'Accounts on this Mac' }),
    null,
  );
  assert.ok(rendered.getByText('No YubiKey enrolled.'));
  assert.ok(rendered.getByText('No security key connected.'));
  assert.ok(
    rendered.getByText('Use a paper key to recover an existing account.'),
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
