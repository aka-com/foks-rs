/**
 * The Teams list.
 *
 * It draws the store rows the rail used to draw, and says a store's abnormal
 * state the same way: a chip at the end of the row and a dimmed row, never a
 * caption under the name. The rows are also the only way into a group's
 * settings now, so where they navigate is pinned here. The Files tab's own
 * rows are covered in `items-screen`'s own tests: it opens on the folder
 * browser directly, with no roots page of its own any more.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';
import type { Bridge } from '../src/bridge';
import { CatalogCoordinator } from '../src/catalog-coordinator';
import type { Location } from '../src/location';
import type { AgentSnapshot, TeamStore } from '../src/model';

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

/** The row a name is drawn in. */
function row(name: string): HTMLElement {
  const node = [
    ...document.querySelectorAll<HTMLElement>('.nav-rows .row'),
  ].find((candidate) => candidate.querySelector('.tt')?.textContent === name);
  assert.ok(node, `no row named ${name}`);
  return node;
}

/** The state chip a row carries, or `null` when the row reports nothing wrong. */
function stateChip(node: HTMLElement): string | null {
  return node.querySelector('.tail .chip.warn')?.textContent ?? null;
}

/** The chip naming this account's role in a Teams row's team. */
function roleChip(node: HTMLElement): string | null {
  return node.querySelector('.tail .chip.role')?.textContent ?? null;
}

/** The Chat/Share pill naming a Teams row's kind. */
function kindChip(node: HTMLElement): string | null {
  return node.querySelector('.tail .chip.kind')?.textContent ?? null;
}

async function fixture(): Promise<AgentSnapshot> {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  return FIXTURE;
}

/** The page as one address and one snapshot draw it. */
interface TeamsOptions {
  snapshot?: AgentSnapshot;
  /** The account creating, inviting and joining act as. */
  store?: string;
  /** The fixture scene, which may open a sheet on mount. */
  scene?: string;
  refresh?: (force?: boolean) => Promise<AgentSnapshot>;
  bridge?: Bridge;
  /** Wraps the mock bridge to observe the commands a row's menu issues. */
  patchBridge?: (base: Bridge) => Bridge;
  mutationError?: (
    error: unknown,
    options?: { report?: boolean },
  ) => Promise<void>;
}

async function teams(
  onNavigate: (location: Location) => void = () => {},
  options: TeamsOptions = {},
) {
  const { TeamsScreen } = (await vite.ssrLoadModule(
    '/src/screens/teams-screen.tsx',
  )) as typeof import('../src/screens/teams-screen');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { ToastController, ToastProvider } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const snapshot = options.snapshot ?? (await fixture());
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(TeamsScreen, {
          snapshot,
          bridge:
            options.bridge ??
            (options.patchBridge
              ? options.patchBridge(mockBridge(snapshot))
              : mockBridge(snapshot)),
          location: {
            kind: 'teams',
            ...(options.store ? { store: options.store } : {}),
          },
          scene: options.scene ?? 'groups',
          onNavigate,
          onRefresh: async () => {},
          onRefreshSnapshot: options.refresh ?? (async () => snapshot),
          onError: (error: unknown) => {
            throw error;
          },
          onMutationError:
            options.mutationError ??
            (async (error: unknown) => {
              throw error;
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

test('Teams centers a spinner while the initial catalog is loading', async () => {
  const snapshot = {
    ...(await fixture()),
    servers: [],
    accounts: [],
    stores: [],
    profileInventory: [],
    catalogProfiles: [],
    profileInventoryStatus: 'unavailable' as const,
  };
  const rendered = await teams(() => {}, { snapshot });
  const loading = rendered.getByRole('status', { name: 'Loading teams' });
  assert.ok(loading.classList.contains('app-loading'));
  assert.ok(loading.classList.contains('body'));
  assert.ok(loading.querySelector('.spin'));
  assert.equal(rendered.queryByText('No teams yet'), null);
});

test('a Teams row in an abnormal state carries the same chip, and opens team settings', async () => {
  const journal: Location[] = [];
  await teams((location) => journal.push(location));
  const homelab = row('Homelab');
  assert.ok(homelab.className.split(' ').includes('off'));
  assert.equal(stateChip(homelab), 'Setup incomplete');
  // Named teams and ad-hoc shares are one list, so a row's own Chat/Share
  // pill says what it is; the caption says only the catalog facts it has.
  assert.equal(kindChip(homelab), 'Share');
  assert.equal(
    homelab.querySelector('.name small')?.textContent,
    'Personal server · 0 members',
  );

  const eng = row('Engineering');
  assert.equal(eng.className, 'row');
  // A normal row's only state is what its shared request count says, once
  // that has been read; the fixture's inbox holds two requests for it.
  await ui.waitFor(() => assert.equal(stateChip(eng), '2 requests'));
  assert.equal(kindChip(eng), 'Chat');
  await ui.act(async () => {
    ui.fireEvent.click(eng);
  });
  assert.deepEqual(journal.at(-1), {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'people',
  });
});

test('a Teams row says how many requests are waiting on it without its page having been opened', async () => {
  const snapshot = await fixture();
  const { manageReason } = (await vite.ssrLoadModule(
    '/src/screens/group-model.ts',
  )) as typeof import('../src/screens/group-model');
  const { INVITATION_ACTIVITY } = (await vite.ssrLoadModule(
    '/src/components/invitation-panel.tsx',
  )) as typeof import('../src/components/invitation-panel');
  const asked: string[] = [];
  await teams(() => {}, {
    snapshot,
    patchBridge: (base) => ({
      ...base,
      invitation: async (profile, account, action, secret) => {
        if (action.action === 'inbox') asked.push(action.team_alias);
        return base.invitation(profile, account, action, secret);
      },
    }),
  });
  // The count is a shared row read for every named team this account can
  // manage, so it is known here, not only after that team's own page loaded.
  await ui.waitFor(() =>
    assert.equal(stateChip(row('Engineering')), '2 requests'),
  );
  assert.equal(kindChip(row('Engineering')), 'Chat');
  // A team the account cannot manage is not asked: there is nothing it
  // could decide.
  const manageable = snapshot.stores.filter(
    (store): store is TeamStore =>
      store.kind === 'team' &&
      store.team_kind === 'named' &&
      manageReason(snapshot, store, 'roster') === undefined,
  );
  assert.ok(
    manageable.length < snapshot.stores.filter((s) => s.kind === 'team').length,
  );
  assert.deepEqual(
    [...asked].sort(),
    manageable.map((store) => store.alias).sort(),
  );

  // Invitation activity for another account leaves these rows alone;
  // activity for this account asks again.
  const before = asked.length;
  await ui.act(async () => {
    window.dispatchEvent(
      new window.CustomEvent(INVITATION_ACTIVITY, {
        detail: { profile: 'acme', account: 'someone-else' },
      }),
    );
    await Promise.resolve();
  });
  assert.equal(asked.length, before);
  const eng = manageable.find((store) => store.id === 'team:eng');
  assert.ok(eng?.kind === 'team');
  await ui.act(async () => {
    window.dispatchEvent(
      new window.CustomEvent(INVITATION_ACTIVITY, {
        detail: { profile: eng.server, account: eng.account },
      }),
    );
    await Promise.resolve();
  });
  await ui.waitFor(() => assert.ok(asked.length > before));
});

test('a Teams row displays the server, member count, and user role', async () => {
  await teams();
  const eng = row('Engineering');
  // The caption is built only from catalog facts: the server and the roster
  // the team's own details call loaded on refresh (every party, the way the
  // team page's own summary counts them). The role this account holds is a
  // chip at the row's end.
  assert.equal(
    eng.querySelector('.name small')?.textContent,
    'Acme · 6 members',
  );
  assert.equal(roleChip(eng), 'Admin');
  assert.equal(
    row('Household').querySelector('.name small')?.textContent,
    'Personal server · 2 members',
  );
  assert.equal(roleChip(row('Household')), 'Owner');
  // No item-readability count is drawn on the list.
  assert.equal(document.body.textContent?.includes('items readable'), false);
});

test('Find teams lists accounts under their servers', async () => {
  const rendered = await teams();
  assert.equal(rendered.queryByRole('menu', { name: 'Find teams' }), null);
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Find teams' }));
  assert.ok(rendered.getByRole('group', { name: 'Personal server' }));
  assert.ok(rendered.getByRole('group', { name: 'Acme' }));
  assert.ok(rendered.getByRole('menuitem', { name: 'Check as satoshi' }));
  assert.ok(rendered.getByRole('menuitem', { name: 'Check as vitalik' }));
  assert.ok(rendered.getByText('Create a team'));
  assert.ok(rendered.getByText('Join a team…'));
});

test('Find teams keeps two accounts on one server distinct', async () => {
  const snapshot = await fixture();
  const account = snapshot.accounts.find(
    (account) => account.store === 'acct:work',
  )!;
  const store = snapshot.stores.find((store) => store.id === account.store)!;
  const rendered = await teams(() => {}, {
    snapshot: {
      ...snapshot,
      accounts: [
        ...snapshot.accounts,
        { ...account, store: 'acct:other', alias: 'other', username: 'alice' },
      ],
      stores: [
        ...snapshot.stores,
        { ...store, id: 'acct:other', account: 'other' },
      ],
    },
  });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Find teams' }));
  const server = rendered.getByRole('group', { name: 'Acme' });
  assert.equal(server.querySelectorAll('button').length, 2);
  assert.ok(server.textContent?.includes('Check as vitalik'));
  assert.ok(server.textContent?.includes('Check as alice'));
});

test('Find teams provides an empty state without an account', async () => {
  const snapshot = await fixture();
  const rendered = await teams(() => {}, {
    snapshot: { ...snapshot, accounts: [], stores: [] },
  });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Find teams' }));
  assert.ok(rendered.getByText('Add an account to find its teams.'));
  assert.equal(rendered.queryByRole('menuitem'), null);
  ui.fireEvent.pointerDown(document.body);
  assert.equal(rendered.queryByRole('menu', { name: 'Find teams' }), null);
});

test('discovery keeps its result in the open menu and across reopening', async () => {
  const rendered = await teams();
  const trigger = rendered.getByRole('button', { name: 'Find teams' });
  ui.fireEvent.click(trigger);
  const check = rendered.getByRole('menuitem', { name: 'Check as vitalik' });
  assert.equal(check.querySelector('[role="status"]')?.textContent, '');
  await ui.act(async () => {
    ui.fireEvent.click(check);
  });
  await ui.waitFor(() =>
    assert.match(
      check.querySelector('[role="status"]')?.textContent ?? '',
      /for vitalik/,
    ),
  );
  assert.ok(rendered.getByRole('menu', { name: 'Find teams' }));
  ui.fireEvent.keyDown(document, { key: 'Escape' });
  assert.equal(rendered.queryByRole('menu', { name: 'Find teams' }), null);
  assert.equal(document.activeElement, trigger);
  ui.fireEvent.click(trigger);
  assert.match(
    rendered.getByRole('group', { name: 'Acme' }).textContent ?? '',
    /for vitalik/,
  );
  assert.equal(document.querySelector('.toasts')?.textContent ?? '', '');
});

/** One discovery failure, as the agent would state it. */
function discoveryFailure(code: string, message: string, retryable: boolean) {
  return {
    code,
    message,
    retryable,
    fatal: false,
    ambiguous: false,
  };
}

test('a discovery failure is rendered only in its result row', async () => {
  const reported: ({ report?: boolean } | undefined)[] = [];
  const rendered = await teams(undefined, {
    patchBridge: (base) => ({
      ...base,
      discoverGroups: async () => {
        throw discoveryFailure('io', 'The server could not be reached.', true);
      },
    }),
    mutationError: async (_error, options) => {
      reported.push(options);
    },
  });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Find teams' }));
  const check = rendered.getByRole('menuitem', { name: 'Check as vitalik' });
  await ui.act(async () => {
    ui.fireEvent.click(check);
  });
  await ui.waitFor(() =>
    assert.equal(
      check.querySelector('[role="status"]')?.textContent,
      'Check failed. The server could not be reached. Try again.',
    ),
  );
  // The row carries the agent's sentence, so the shell is told not to toast
  // it as well.
  assert.deepEqual(reported, [{ report: false }]);
  assert.equal(document.querySelector('.toasts')?.textContent ?? '', '');
});

test('nonretryable and cancelled discovery failures render appropriate results', async () => {
  let outcome = discoveryFailure(
    'capability-denied',
    'This account may not list teams.',
    false,
  );
  const rendered = await teams(undefined, {
    patchBridge: (base) => ({
      ...base,
      discoverGroups: async () => {
        throw outcome;
      },
    }),
    mutationError: async () => {},
  });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Find teams' }));
  const check = rendered.getByRole('menuitem', { name: 'Check as vitalik' });
  await ui.act(async () => {
    ui.fireEvent.click(check);
  });
  await ui.waitFor(() =>
    assert.equal(
      check.querySelector('[role="status"]')?.textContent,
      'Check failed. This account may not list teams.',
    ),
  );
  outcome = discoveryFailure('cancelled', 'The check was cancelled.', false);
  await ui.act(async () => {
    ui.fireEvent.click(check);
  });
  await ui.waitFor(() =>
    assert.equal(check.querySelector('[role="status"]')?.textContent, ''),
  );
});

/**
 * An item of one row's open menu, found by the label it ends with. The menu is
 * named for the row it belongs to, so an item is never read off another row's.
 */
function menuItem(owner: string, label: string): HTMLButtonElement {
  const menu = document.querySelector(
    `.menu[aria-label="Actions for ${owner}"]`,
  );
  assert.ok(menu, `no open menu for ${owner}`);
  const node = [...menu.querySelectorAll<HTMLButtonElement>('button')].find(
    (candidate) => (candidate.textContent ?? '').trim().endsWith(label),
  );
  assert.ok(node, `no menu item labeled ${label} for ${owner}`);
  return node;
}

/**
 * One of the two Add people choices in a row's open menu, found by its label,
 * since the item's text runs on into the line that explains it.
 */
function choiceItem(owner: string, label: string): HTMLButtonElement {
  const menu = document.querySelector(
    `.menu[aria-label="Actions for ${owner}"]`,
  );
  assert.ok(menu, `no open menu for ${owner}`);
  const node = [...menu.querySelectorAll<HTMLButtonElement>('button')].find(
    (candidate) =>
      candidate.querySelector('.menu-choice > span')?.textContent === label,
  );
  assert.ok(node, `no Add people choice labeled ${label} for ${owner}`);
  return node;
}

/** Whether an item is present but does nothing. */
function inert(node: HTMLButtonElement): boolean {
  return node.getAttribute('aria-disabled') === 'true';
}

/** Open one row's menu, closing whichever was open. */
function openRowMenu(name: string): void {
  ui.fireEvent.keyDown(document, { key: 'Escape' });
  const trigger = document.querySelector<HTMLButtonElement>(
    `button[aria-label="Actions for ${name}"]`,
  );
  assert.ok(trigger, `no menu trigger for ${name}`);
  ui.fireEvent.click(trigger);
  // One menu at a time: the row opened before this one is closed.
  assert.equal(document.querySelectorAll('.menu').length, 1);
  assert.ok(document.querySelector(`.menu[aria-label="Actions for ${name}"]`));
}

test('a Teams row menu says why an action does not apply', async () => {
  await teams();
  // This account is an Admin of Engineering, so the roster actions apply.
  openRowMenu('Engineering');
  // The list asks the add question the way the team page does: two choices,
  // each with its own reason, and its own line saying how it works.
  const user = choiceItem('Engineering', 'Add a user…');
  assert.equal(inert(user), false);
  assert.equal(
    user.querySelector('.menu-choice small')?.textContent,
    'Invite by their username',
  );
  const federated = choiceItem(
    'Engineering',
    'Add a team from another server…',
  );
  assert.equal(inert(federated), false);
  assert.equal(
    federated.querySelector('.menu-choice small')?.textContent,
    'Add a remote team via federation',
  );
  assert.equal(ui.screen.queryByRole('menuitem', { name: 'Leave…' }), null);

  // An ad-hoc share has no membership to change, and its setup is unfinished.
  openRowMenu('Homelab');
  const add = choiceItem('Homelab', 'Add a user…');
  assert.equal(inert(add), true);
  assert.equal(
    add.getAttribute('title'),
    'Memberships can’t be changed in an ad-hoc team.',
  );
  assert.equal(
    inert(choiceItem('Homelab', 'Add a team from another server…')),
    true,
  );
  assert.equal(inert(menuItem('Homelab', 'Finish setup…')), false);
  // Finishing cannot succeed for every stuck creation, so the way out sits
  // beside it — and only on a row whose setup never finished.
  assert.equal(inert(menuItem('Homelab', 'Remove team…')), false);
  // Copying the team ID is a local fact, so it applies throughout.
  assert.equal(inert(menuItem('Homelab', 'Copy team ID')), false);
  openRowMenu('Engineering');
  assert.equal(
    [
      ...document
        .querySelectorAll<HTMLButtonElement>(
          '.menu[aria-label="Actions for Engineering"] button',
        )
        .values(),
    ].some((node) => (node.textContent ?? '').trim() === 'Remove team…'),
    false,
  );
});

test('a Teams row removes a creation that finishing cannot fix', async () => {
  const abandoned: string[] = [];
  const rendered = await teams(() => {}, {
    patchBridge: (base) => ({
      ...base,
      abandonGroupCreation: async (storeId: string) => {
        abandoned.push(storeId);
        return base.abandonGroupCreation(storeId);
      },
    }),
  });
  openRowMenu('Homelab');
  await ui.act(async () => {
    ui.fireEvent.click(menuItem('Homelab', 'Remove team…'));
  });
  const confirm = rendered.getByRole('button', { name: 'Remove team' });
  assert.equal(confirm.hasAttribute('disabled'), true);
  const field = document.querySelector<HTMLInputElement>(
    'input[placeholder="type homelab"]',
  );
  assert.ok(field);
  await ui.act(async () => {
    ui.fireEvent.change(field, { target: { value: 'homelab' } });
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Remove team' }));
  });
  assert.deepEqual(abandoned, ['team:homelab']);
});

test('a roster failure gives the Teams row menu its own reasons', async () => {
  const snapshot = await fixture();
  await teams(() => {}, {
    snapshot: {
      ...snapshot,
      // A roster that could not be read leaves no parties behind, so the
      // fixture's are dropped with it.
      parties: snapshot.parties.filter((party) => party.store !== 'team:eng'),
      groupDetailFailures: [
        {
          store: 'team:eng',
          source: 'roster',
          code: 'unavailable',
          message: 'The group roster could not be read from the agent.',
          retryable: true,
        },
      ],
    },
  });
  // The roster failed to load, so the member count is dropped rather than
  // stated as zero; with it goes the caller's own role, since that party list
  // is what it would have come from.
  assert.equal(
    row('Engineering').querySelector('.name small')?.textContent,
    'Acme',
  );
  const unread = 'The roster could not be read. Refresh before making changes.';
  openRowMenu('Engineering');
  // Both ways in are refused for the same reason — the role that would permit
  // either is a roster fact, so an unread roster settles nothing about them —
  // and each choice states it for itself.
  const add = choiceItem('Engineering', 'Add a user…');
  assert.equal(inert(add), true);
  assert.equal(add.getAttribute('title'), unread);
  assert.equal(
    inert(choiceItem('Engineering', 'Add a team from another server…')),
    true,
  );
});

test('an unavailable account explains why discovery is disabled', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/lease.ts',
  )) as typeof import('../src/model/lease');
  const rendered = await teams(() => {}, {
    snapshot: applyLease(await fixture(), 'lapsed'),
  });
  // A store whose access is stopped still gets its full caption: the lapse
  // is a fact about the server, not about the roster the catalog already
  // holds, so the chip beside it states the problem rather than replacing it.
  const eng = row('Engineering');
  assert.equal(stateChip(eng), 'Check-in expired');
  assert.equal(kindChip(eng), 'Chat');
  assert.equal(
    eng.querySelector('.name small')?.textContent,
    'Acme · 6 members',
  );
  assert.equal(roleChip(eng), 'Admin');
  const trigger = rendered.getByRole('button', { name: 'Find teams' });
  assert.equal(trigger.getAttribute('aria-expanded'), 'false');
  ui.fireEvent.click(trigger);
  const check = rendered.getByRole('menuitem', { name: /Check as vitalik/ });
  assert.equal(check.getAttribute('aria-disabled'), 'true');
  assert.ok(check.querySelector('small')?.textContent);
  ui.fireEvent.click(check);
  assert.ok(rendered.getByRole('menu', { name: 'Find teams' }));
});

/** The server and account the create sheet is seeded to. */
function seededAccount(): string | null {
  return (
    document.querySelector(
      '[role="radiogroup"][aria-label="Server and account"] [aria-checked="true"] .t b',
    )?.textContent ?? null
  );
}

test('creating and joining act as the account the address names', async () => {
  const created = await teams(() => {}, {
    store: 'acct:work',
    scene: 'create',
  });
  // The sheet opens on mount, on the account the address named.
  assert.equal(
    created.getByRole('heading', { name: 'Create a team' }).textContent,
    'Create a team',
  );
  // The account choice identifies the server without a repeated subtitle.
  assert.equal(
    document.querySelector('.sheet .hd small')?.textContent,
    undefined,
  );
  assert.equal(seededAccount(), 'Acme');
  // The action label stays stable while the group name is edited.
  assert.ok(created.getByRole('button', { name: 'Create team' }));
  ui.cleanup();

  // Another account in the address seeds the sheet to that one instead.
  await teams(() => {}, { store: 'acct:personal', scene: 'create' });
  assert.equal(
    document.querySelector('.sheet .hd small')?.textContent,
    undefined,
  );
  assert.equal(seededAccount(), 'Personal server');
  ui.cleanup();

  const joining = await teams(() => {}, {
    store: 'acct:work',
    scene: 'join',
  });
  assert.ok(joining.getByRole('heading', { name: 'Join a team' }));
  // The Teams page already names the account, so the sheet does not.
  assert.equal(document.querySelector('.sheet .hd small'), null);
});

test('a created group closes its sheet when only the post-write refresh fails', async () => {
  let reads = 0;
  const rendered = await teams(() => {}, {
    scene: 'create',
    refresh: async () => {
      reads++;
      throw new Error('refresh unavailable');
    },
    mutationError: async () => {
      assert.fail('applied creation must not enter mutation failure recovery');
    },
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Create team' }));
  });
  await ui.waitFor(() => assert.equal(rendered.queryByRole('dialog'), null));
  assert.equal(reads, 1);
});

test('group creation replaces a cancelled foreground catalog load without repeating the write', async () => {
  const snapshot = await fixture();
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const base = mockBridge(snapshot);
  const { loadSnapshot } = (await vite.ssrLoadModule(
    '/src/bridge.ts',
  )) as typeof import('../src/bridge');
  let writes = 0;
  let reads = 0;
  let cancelRead!: (error: unknown) => void;
  const published: AgentSnapshot[] = [];
  const coordinator = new CatalogCoordinator<AgentSnapshot>(
    () => {
      reads++;
      return reads === 1
        ? new Promise((_, reject) => {
            cancelRead = reject;
          })
        : loadSnapshot(base, snapshot);
    },
    (value) => {
      published.push(value);
    },
  );
  const foreground = coordinator.refresh();
  // Observe a potential rejection before releasing the read barrier.
  const foregroundResult = foreground.catch(() => undefined);
  await ui.waitFor(() => assert.equal(reads, 1));
  const forces: boolean[] = [];
  const destinations: Location[] = [];
  const rendered = await teams((location) => destinations.push(location), {
    scene: 'create',
    bridge: {
      ...base,
      createGroup: async (input) => {
        writes++;
        return base.createGroup(input);
      },
    },
    refresh: (force = false) => {
      forces.push(force);
      return coordinator.refresh(force);
    },
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Create team' }));
  });
  await ui.waitFor(() => assert.deepEqual(forces, [true]));
  await ui.act(async () => {
    cancelRead({
      code: 'cancelled',
      message: 'The request was cancelled.',
      retryable: true,
      ambiguous: false,
      fatal: false,
    });
    await foregroundResult;
  });
  await ui.waitFor(() => assert.equal(rendered.queryByRole('dialog'), null));
  assert.equal(writes, 1);
  assert.equal(reads, 2);
  assert.equal(published.length, 1);
  assert.ok(published[0].stores.some((store) => store.id === 'team:platform'));
  assert.deepEqual(destinations, [{ kind: 'store', ref: 'team:platform' }]);
  assert.equal(rendered.queryByText(/Updated data could not be loaded/), null);
});
