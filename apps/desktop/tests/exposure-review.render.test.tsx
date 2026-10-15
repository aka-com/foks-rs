/**
 * The exposure review in the UI: the remove, lower-role and remove-device
 * sheets list what the party could read before the write and keep the list
 * once it succeeds; the team page and Devices draw a band for an unmarked
 * review; the review sheet opens items and records each entry's mark.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';

import { installDom } from './lib/dom-harness';
import { loadTestShell } from './lib/test-shell';
import type { Bridge } from '../src/bridge';
import type { AgentSnapshot, Item } from '../src/model';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');

test.before(async () => {
  ui = await import('@testing-library/react');
});
test.beforeEach(() => window.localStorage.clear());
test.afterEach(() => {
  ui.cleanup();
  window.localStorage.clear();
});

const STORAGE_KEY = 'foks.exposureReviews';

interface StoredReview {
  scope: string;
  party: string;
  name: string;
  cause: string;
  entries: { store: string; dirent: string; version: number; mark?: string }[];
}

function stored(): StoredReview[] {
  return JSON.parse(
    window.localStorage.getItem(STORAGE_KEY) ?? '[]',
  ) as StoredReview[];
}

async function fixture(): Promise<AgentSnapshot> {
  const { FIXTURE } = await import('../src/mock/fixture');
  return FIXTURE;
}

async function providers(child: ReturnType<typeof createElement>) {
  const { ToastController, ToastProvider } = await import('../kit/toasts');
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  return createElement(OverlayProvider, {
    backgroundRef: { current: null },
    portalRoot,
    children: createElement(ToastProvider, {
      controller: new ToastController(),
      children: child,
    }),
  });
}

async function teamPage(
  snapshot: AgentSnapshot,
  ref: string,
  opened: Item[] = [],
  {
    overrides = {},
    failures,
    tab = 'settings',
  }: {
    overrides?: Partial<Bridge>;
    /** Collects mutation failures; without it, a failure fails the test. */
    failures?: unknown[];
    tab?: 'settings' | 'people';
  } = {},
) {
  const { GroupSettingsScreen } = await import('../src/screens/groups-screen');
  const TestShell = await loadTestShell();
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const bridge = { ...mockBridge(snapshot), ...overrides };
  const rendered = ui.render(
    await providers(
      createElement(TestShell, {
        snapshot,
        bridge,
        commandError: (error: unknown) => {
          throw error;
        },
        mutationError: async (error: unknown) => {
          if (!failures) throw error;
          failures.push(error);
        },
        children: createElement(GroupSettingsScreen, {
          location: { kind: 'group-settings', ref, tab },
          onNavigate: () => {},
          onOpenItem: (item: Item) => opened.push(item),
        }),
      }),
    ),
  );
  await ui.act(async () => {});
  return rendered;
}

/** Opens the Settings tab's Remove… menu and picks `username`. */
async function openRemoval(
  rendered: Awaited<ReturnType<typeof teamPage>>,
  username: string,
) {
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Remove…' }));
  });
  const choice = [
    ...document.querySelectorAll<HTMLButtonElement>('.menu button'),
  ].find((node) => node.querySelector('.t b')?.textContent === username);
  assert.ok(choice, `no removal choice for ${username}`);
  await ui.act(async () => {
    ui.fireEvent.click(choice);
  });
  return rendered.getByRole('alertdialog', {
    name: `Remove ${username} from Household?`,
  });
}

const listed = (list: HTMLElement): string[] =>
  [...list.querySelectorAll('.exposure-row .t b')].map(
    (node) => node.textContent ?? '',
  );

test('removing a member lists what they could read and keeps it for review', async () => {
  const snapshot = await fixture();
  const opened: Item[] = [];
  const rendered = await teamPage(snapshot, 'team:household', opened);
  const dialog = await openRemoval(rendered, 'sam');
  // Passwords first, then Documents.
  const list = ui.within(dialog).getByRole('list', {
    name: 'Items sam could read',
  });
  assert.deepEqual(listed(list), [
    'netflix',
    'guest-password',
    'emergency.pdf',
  ]);
  assert.match(dialog.textContent ?? '', /the list may be\s+incomplete/);
  assert.deepEqual(stored(), []);

  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Remove and rekey' }),
    );
  });
  const [review] = stored();
  assert.ok(review);
  assert.equal(review.scope, 'team:household');
  assert.equal(review.name, 'sam');
  assert.equal(review.cause, 'removed');
  assert.deepEqual(
    review.entries.map((entry) => entry.dirent),
    [
      '0000000000000000000000000000000e',
      '0000000000000000000000000000000c',
      '0000000000000000000000000000000d',
    ],
  );

  // The page now leads to the review.
  const band = await rendered.findByRole('group', {
    name: 'Review items sam could read',
  });
  assert.match(band.textContent ?? '', /3 of 3 items sam could\s*read are/);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(band).getByRole('button', { name: 'Review…' }),
    );
  });
  let sheet = rendered.getByRole('dialog', {
    name: 'Review items sam could read',
  });
  assert.deepEqual(
    listed(ui.within(sheet).getByRole('list', { name: 'Passwords' })),
    ['netflix', 'guest-password'],
  );
  assert.deepEqual(
    listed(ui.within(sheet).getByRole('list', { name: 'Documents' })),
    ['emergency.pdf'],
  );
  assert.equal(ui.within(sheet).getAllByText('Unchanged').length, 3);

  // Open hands the item to Files and closes the sheet.
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Open netflix' }),
    );
  });
  assert.deepEqual(
    opened.map((item) => item.path),
    ['/streaming/netflix'],
  );
  assert.equal(
    rendered.queryByRole('dialog', { name: 'Review items sam could read' }),
    null,
  );

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Review…' }));
  });
  sheet = rendered.getByRole('dialog', { name: 'Review items sam could read' });
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Mark netflix rotated' }),
    );
    ui.fireEvent.click(
      ui
        .within(sheet)
        .getByRole('button', { name: 'Mark guest-password rotated' }),
    );
  });
  assert.equal(ui.within(sheet).getAllByText('Rotated').length >= 2, true);
  assert.match(
    rendered.getByRole('group', { name: 'Review items sam could read' })
      .textContent ?? '',
    /1 of 3 items sam could\s*read is/,
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui
        .within(sheet)
        .getByRole('button', { name: 'Mark emergency.pdf not needed' }),
    );
  });
  // Every entry is marked: the band is gone, and Done drops the review.
  assert.equal(
    rendered.queryByRole('group', { name: 'Review items sam could read' }),
    null,
  );
  assert.equal(stored().length, 1);
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(sheet).getByRole('button', { name: 'Done' }));
  });
  assert.deepEqual(stored(), []);
});

test('an item written after the removal reads as changed, not rotated', async () => {
  const snapshot = await fixture();
  window.localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify([
      {
        scope: 'team:household',
        party: 'p',
        name: 'sam',
        cause: 'removed',
        at: Date.UTC(2026, 0, 2),
        entries: [
          {
            store: 'team:household',
            dirent: '0000000000000000000000000000000e',
            version: 0,
          },
          {
            store: 'team:household',
            dirent: 'ffffffffffffffffffffffffffffffff',
            version: 1,
          },
        ],
      },
    ]),
  );
  const rendered = await teamPage(snapshot, 'team:household');
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Review…' }));
  });
  const sheet = rendered.getByRole('dialog', {
    name: 'Review items sam could read',
  });
  assert.ok(ui.within(sheet).getByText('Changed since removal'));
  // An entry whose item is no longer listed has no Open, but can be marked.
  const missing = ui.within(sheet).getByRole('list', { name: 'Not listed' });
  assert.deepEqual(listed(missing), ['Item not listed']);
  assert.equal(
    ui.within(missing).queryByRole('button', { name: 'Open Item not listed' }),
    null,
  );
  // Clear review drops it whatever its entries say.
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Clear review' }),
    );
  });
  assert.deepEqual(stored(), []);
});

test('the removal waits while the team’s items are still being read', async () => {
  const fixtureSnapshot = await fixture();
  const snapshot: AgentSnapshot = {
    ...fixtureSnapshot,
    storeInventory: fixtureSnapshot.storeInventory.map((entry) =>
      entry.store === 'team:household'
        ? { ...entry, status: 'loading' as const }
        : entry,
    ),
  };
  const rendered = await teamPage(snapshot, 'team:household');
  const dialog = await openRemoval(rendered, 'sam');
  assert.ok(ui.within(dialog).getByText('Listing the items sam can read…'));
  const remove = ui
    .within(dialog)
    .getByRole('button', { name: 'Remove and rekey' });
  assert.ok(remove instanceof HTMLButtonElement);
  assert.equal(remove.disabled, true);
});

test('lowering a role lists only the items the new role no longer admits', async () => {
  const snapshot = await fixture();
  const { GroupSheet } = await import('../src/screens/groups/group-sheet');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const TestShell = await loadTestShell();
  const store = snapshot.stores.find((entry) => entry.id === 'team:eng');
  assert.ok(store);
  const priya = snapshot.parties.find(
    (party) => party.store === 'team:eng' && party.username === 'priya.n',
  );
  assert.ok(priya);
  const rendered = ui.render(
    await providers(
      createElement(TestShell, {
        snapshot,
        bridge: mockBridge(snapshot),
        children: createElement(GroupSheet, {
          store,
          sheet: 'demote',
          target: priya,
          onClose: () => {},
          onApplied: async () => {},
          onMutationError: async (error: unknown) => {
            throw error;
          },
        }),
      }),
    ),
  );
  await ui.act(async () => {});
  const list = rendered.getByRole('list', {
    name: 'Items priya.n will no longer read',
  });
  assert.deepEqual(listed(list), ['production-token']);
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Change role' }));
  });
  const [review] = stored();
  assert.ok(review);
  assert.equal(review.cause, 'lowered');
  assert.equal(review.party, priya.party_id_hex);
  assert.deepEqual(
    review.entries.map((entry) => [entry.dirent, entry.version]),
    [['00000000000000000000000000000008', 12]],
  );
});

/** Devices for the personal account, with the Remove device sheet open. */
async function removeDeviceSheet(snapshot: AgentSnapshot) {
  const { DevicesScreen } = await import('../src/screens/devices-screen');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const TestShell = await loadTestShell();
  const device = snapshot.devices[1]?.id_hex.replace(/^02/, '04');
  assert.ok(device);
  const page = async (at?: string) => {
    const bridge = mockBridge(snapshot);
    return providers(
      createElement(TestShell, {
        snapshot,
        bridge,
        commandError: (error: unknown) => {
          throw error;
        },
        mutationError: async (error: unknown) => {
          throw error;
        },
        children: createElement(DevicesScreen, {
          location: {
            kind: 'devices',
            store: 'acct:personal',
            ...(at ? { device: at } : {}),
          },
          scene: 'devices',
          onNavigate: () => {},
        }),
      }),
    );
  };
  const rendered = ui.render(await page(device));
  await ui.act(async () => {
    await Promise.resolve();
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      await rendered.findByRole('button', { name: 'Remove this device…' }),
    );
  });
  const input = document.querySelector<HTMLInputElement>('.sheet input');
  assert.ok(input);
  const name = input.placeholder.replace(/^type /, '');
  const remove = async () => {
    ui.fireEvent.change(input, { target: { value: name } });
    await ui.act(async () => {
      ui.fireEvent.click(
        rendered.getByRole('button', { name: 'Remove device' }),
      );
    });
  };
  return { rendered, device, name, page, remove };
}

test('removing a device lists every item of its account and keeps the review', async () => {
  const snapshot = await fixture();
  const { rendered, device, name, page, remove } =
    await removeDeviceSheet(snapshot);
  const list = rendered.getByRole('list', { name: `Items ${name} could read` });
  // Six rows, Passwords first, and a count of the rest.
  assert.deepEqual(listed(list).slice(0, 4), [
    'fastmail.com',
    'github.com',
    'netflix',
    'guest-password',
  ]);
  assert.equal(list.querySelectorAll('.exposure-row').length, 7);
  assert.equal(
    list.querySelector('.exposure-row.more')?.textContent,
    'and 3 more items',
  );
  await remove();
  const [review] = stored();
  assert.ok(review);
  assert.equal(review.scope, 'acct:personal');
  assert.equal(review.party, device);
  assert.equal(review.cause, 'device-removed');
  assert.equal(review.entries.length, 9);

  rendered.rerender(await page());
  await ui.act(async () => {
    await Promise.resolve();
  });
  assert.ok(
    await rendered.findByRole('group', {
      name: `Review items ${name} could read`,
    }),
  );
});

test('a team whose items cannot be listed is named, and the account’s items are kept', async () => {
  const fixtureSnapshot = await fixture();
  const snapshot: AgentSnapshot = {
    ...fixtureSnapshot,
    storeInventory: fixtureSnapshot.storeInventory.map((entry) =>
      entry.store === 'team:household'
        ? { ...entry, status: 'unavailable' as const }
        : entry,
    ),
  };
  const { rendered, name, remove } = await removeDeviceSheet(snapshot);
  const sheet = rendered.getByRole('alertdialog');
  assert.match(
    sheet.textContent ?? '',
    /Items in Household could not be listed and are not included\./,
  );
  const list = rendered.getByRole('list', { name: `Items ${name} could read` });
  assert.equal(listed(list).includes('netflix'), false);
  await remove();
  const [review] = stored();
  assert.ok(review);
  assert.deepEqual(
    [...new Set(review.entries.map((entry) => entry.store))],
    ['acct:personal'],
  );
  assert.equal(review.entries.length, 6);
});

test('a removal that is refused keeps no review; one with an unknown outcome does', async () => {
  const snapshot = await fixture();
  const refusal = {
    code: 'permission-denied',
    message: 'The server refused the change.',
    retryable: false,
    ambiguous: false,
    fatal: false,
  };
  for (const [error, kept] of [
    [refusal, 0],
    [
      {
        ...refusal,
        code: 'agent-lost',
        message: 'The agent stopped.',
        ambiguous: true,
      },
      1,
    ],
  ] as const) {
    const failures: unknown[] = [];
    const rendered = await teamPage(snapshot, 'team:household', [], {
      overrides: {
        removeGroupMember: async () => {
          throw error;
        },
      },
      failures,
    });
    const dialog = await openRemoval(rendered, 'sam');
    await ui.act(async () => {
      ui.fireEvent.click(
        ui.within(dialog).getByRole('button', { name: 'Remove and rekey' }),
      );
    });
    assert.equal(failures.length, 1);
    assert.equal(stored().length, kept);
    ui.cleanup();
    window.localStorage.clear();
  }
});

test('marking an entry keeps focus on its row, and closing drops a fully marked review', async () => {
  const snapshot = await fixture();
  window.localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify([
      {
        scope: 'team:household',
        party: 'p',
        name: 'sam',
        cause: 'removed',
        at: Date.UTC(2026, 0, 2),
        entries: [
          {
            store: 'team:household',
            dirent: '0000000000000000000000000000000e',
            version: 0,
          },
        ],
      },
    ]),
  );
  const rendered = await teamPage(snapshot, 'team:household', []);
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Review…' }));
  });
  const sheet = rendered.getByRole('dialog', {
    name: 'Review items sam could read',
  });
  const rotated = ui
    .within(sheet)
    .getByRole('button', { name: 'Mark netflix rotated' });
  rotated.focus();
  await ui.act(async () => {
    ui.fireEvent.click(rotated);
  });
  assert.equal(
    document.activeElement,
    ui
      .within(sheet)
      .getByRole('button', { name: 'Clear the Rotated mark on netflix' }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(document.activeElement as HTMLElement);
  });
  assert.equal(
    document.activeElement,
    ui.within(sheet).getByRole('button', { name: 'Open netflix' }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Mark netflix not needed' }),
    );
  });
  assert.equal(stored().length, 1);
  // Escape closes the sheet as Done does, and drops the marked review.
  await ui.act(async () => {
    ui.fireEvent.keyDown(document.activeElement ?? sheet, { key: 'Escape' });
  });
  assert.equal(
    rendered.queryByRole('dialog', { name: 'Review items sam could read' }),
    null,
  );
  assert.deepEqual(stored(), []);
});

test('removing a federated team lists what it could read and keeps it for review', async () => {
  const fixtureSnapshot = await fixture();
  const snapshot: AgentSnapshot = {
    ...fixtureSnapshot,
    federation: fixtureSnapshot.federation.map((entry) => ({
      ...entry,
      active: true,
    })),
  };
  const homelab = snapshot.parties.find(
    (party) =>
      party.store === 'team:eng' && party.team_name?.startsWith('homelab'),
  );
  assert.ok(homelab);
  const rendered = await teamPage(snapshot, 'team:eng', [], { tab: 'people' });
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Actions for homelab' }),
    );
  });
  const remove = [
    ...document.querySelectorAll<HTMLButtonElement>('.menu button'),
  ].find((node) =>
    (node.textContent ?? '').trim().endsWith('Remove federated team…'),
  );
  assert.ok(remove);
  await ui.act(async () => {
    ui.fireEvent.click(remove);
  });
  const dialog = rendered.getByRole('alertdialog', {
    name: 'Remove homelab from Engineering?',
  });
  const list = ui
    .within(dialog)
    .getByRole('list', { name: 'Items homelab could read' });
  assert.equal(listed(list).includes('production-token'), false);
  assert.ok(listed(list).length > 0);
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(dialog).getByRole('checkbox'));
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Remove and rekey' }),
    );
  });
  const [review] = stored();
  assert.ok(review);
  assert.equal(review.scope, 'team:eng');
  assert.equal(review.party, homelab.party_id_hex);
  assert.equal(review.name, 'homelab');
  assert.equal(review.cause, 'removed');
  assert.equal(review.entries.length, listed(list).length);
});
