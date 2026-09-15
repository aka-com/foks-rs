/**
 * The Files roots page and the Teams list.
 *
 * Both draw the store rows the rail used to draw, and both say a store's
 * abnormal state the same way: a chip at the end of the row and a dimmed row,
 * never a caption under the name. The rows are also the only way into an item
 * page and a group's settings now, so where they navigate is pinned here.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';
import type { Location } from '../src/location';
import type { AgentSnapshot } from '../src/model';

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

/** The chip naming the role this Mac's account holds in that group. */
function roleChip(node: HTMLElement): string | null {
  return node.querySelector('.tail .chip.role')?.textContent ?? null;
}

async function fixture(): Promise<AgentSnapshot> {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  return FIXTURE;
}

async function files(onNavigate: (location: Location) => void = () => {}) {
  const { FilesScreen } = (await vite.ssrLoadModule(
    '/src/screens/files-screen.tsx',
  )) as typeof import('../src/screens/files-screen');
  const snapshot = await fixture();
  return ui.render(createElement(FilesScreen, { snapshot, onNavigate }));
}

/** The page as one address and one snapshot draw it. */
interface TeamsOptions {
  snapshot?: AgentSnapshot;
  /** The account creating, inviting and joining act as. */
  store?: string;
  /** The fixture scene, which may open a sheet on mount. */
  scene?: string;
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
          bridge: mockBridge(snapshot),
          location: {
            kind: 'teams',
            ...(options.store ? { store: options.store } : {}),
          },
          scene: options.scene ?? 'groups',
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
    }),
  );
  await ui.act(async () => {
    await Promise.resolve();
  });
  return rendered;
}

test('a Files row in an abnormal state carries a chip and is dimmed', async () => {
  await files();
  // Homelab's group reports setup incomplete in the fixture.
  const homelab = row('Homelab');
  assert.ok(homelab.className.split(' ').includes('off'));
  assert.equal(
    homelab.querySelector('.tail .chip')?.textContent,
    'Setup incomplete',
  );
  // The state is the chip, so the caption says only what the store is.
  assert.equal(homelab.querySelector('.name small')?.textContent, 'Share');

  const personal = row('Personal');
  assert.equal(personal.className, 'row');
  assert.equal(personal.querySelector('.tail .chip'), null);
  assert.equal(
    personal.querySelector('.name small')?.textContent,
    'Vault · foks.example.net',
  );
});

test('a Files row draws the group its own mark, and a vault its glyph', async () => {
  await files();
  // One group, one mark: the same initial over the same colour that the Teams
  // row and the group's own page draw.
  const mark = row('Engineering').querySelector('.kico.group');
  assert.ok(mark);
  assert.equal(mark.textContent, 'E');
  // The initial stands for the name beside it, so it is not read out twice.
  assert.equal(mark.getAttribute('aria-hidden'), 'true');
  // A vault is not a group: it keeps the vault glyph over its own hue.
  const vault = row('Personal');
  assert.equal(vault.querySelector('.kico'), null);
  assert.ok(vault.querySelector('.kic'));
});

test('the Files rows open All items and the store they name', async () => {
  const journal: Location[] = [];
  await files((location) => journal.push(location));
  ui.fireEvent.click(row('All items'));
  assert.deepEqual(journal.at(-1), { kind: 'all' });
  ui.fireEvent.click(row('Engineering'));
  assert.deepEqual(journal.at(-1), { kind: 'store', ref: 'team:eng' });
});

test('a Teams row in an abnormal state carries the same chip, and opens group settings', async () => {
  const journal: Location[] = [];
  await teams((location) => journal.push(location));
  const homelab = row('Homelab');
  assert.ok(homelab.className.split(' ').includes('off'));
  assert.equal(stateChip(homelab), 'Setup incomplete');
  // The Shares section above already says what the object is, so the caption
  // says only where it lives; the chip beside it says what is wrong.
  assert.equal(
    homelab.querySelector('.name small')?.textContent,
    'foks.example.net',
  );

  const eng = row('Engineering');
  assert.equal(eng.className, 'row');
  assert.equal(stateChip(eng), null);
  await ui.act(async () => {
    ui.fireEvent.click(eng);
  });
  assert.deepEqual(journal.at(-1), {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'people',
  });
});

test('a Teams row says the server, the roster summary and your role', async () => {
  await teams();
  const eng = row('Engineering');
  // One account per server on this Mac, so the server says all the caption
  // has to; the account is named only where two hold accounts on the same
  // server, and the kind only where no section label already says it.
  assert.equal(
    eng.querySelector('.name small')?.textContent,
    'foks.acme-corp.com',
  );
  // The roster the group's own details call loaded on refresh, split the way
  // the group page splits it: people, machines and admitted groups.
  assert.equal(
    eng.querySelector('.tail .summary')?.textContent,
    '4 people · 1 machine · 1 group',
  );
  // The role is bare: "Admin", never "you are Admin".
  assert.equal(roleChip(eng), 'Admin');
  assert.equal(roleChip(row('Household')), 'Owner');
  // No item-readability count is drawn on the list.
  assert.equal(document.body.textContent?.includes('items readable'), false);
});

test('the Teams page carries one check row per account store', async () => {
  const rendered = await teams();
  const personal = row('foks.example.net');
  assert.equal(
    personal.querySelector('.name small')?.textContent,
    'as satoshi · personal',
  );
  const work = row('foks.acme-corp.com');
  assert.equal(
    work.querySelector('.name small')?.textContent,
    'as vitalik · work',
  );
  // One check and one invite entry for each account, and no repeated
  // "Needs attention" list: the rows carry each store's state.
  assert.equal(rendered.getAllByText('Check for groups').length, 2);
  assert.equal(rendered.getAllByText('Invite someone…').length, 2);
  assert.equal(rendered.queryByText('Needs attention'), null);
  assert.ok(rendered.getByText('Create a group'));
  assert.ok(rendered.getByText('Join a group…'));
});

test('checking an account store reports its result on that row', async () => {
  const rendered = await teams();
  const work = row('foks.acme-corp.com');
  const status = work.querySelector('[role="status"]');
  assert.ok(status, 'the live region exists before the result arrives');
  assert.equal(status.textContent, '');
  const check = [...work.querySelectorAll('button')].find(
    (button) => button.textContent === 'Check for groups',
  );
  assert.ok(check);
  await ui.act(async () => {
    ui.fireEvent.click(check);
  });
  await ui.waitFor(() =>
    assert.ok(
      row('foks.acme-corp.com').querySelector('.tail .summary')?.textContent,
    ),
  );
  const result =
    row('foks.acme-corp.com').querySelector('.tail .summary')?.textContent;
  assert.ok(
    result?.includes('for vitalik'),
    `the check row reported "${result ?? ''}"`,
  );
  assert.equal(rendered.queryByText('Find groups'), null);
  // The row is the only report: a toast repeating the same sentence would say
  // it twice.
  assert.equal(document.querySelector('.toasts')?.textContent ?? '', '');
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
  assert.ok(node, `no menu item labelled ${label} for ${owner}`);
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
  assert.equal(
    inert(menuItem('Engineering', 'Add someone on foks.acme-corp.com…')),
    false,
  );
  assert.equal(inert(menuItem('Engineering', 'Add a group…')), false);
  // Leaving has no command at all, and the reason names who can remove you.
  const leave = menuItem('Engineering', 'Leave…');
  assert.equal(inert(leave), true);
  assert.equal(
    leave.getAttribute('title'),
    'To leave this group, ask sam.ortiz or priya.n to remove your account.',
  );

  // The sole owner of a loaded roster gets the other sentence.
  openRowMenu('Household');
  assert.equal(
    menuItem('Household', 'Leave…').getAttribute('title'),
    'As the sole owner, you must transfer ownership or delete the group to leave.',
  );

  // An ad-hoc share has no membership to change, and its setup is unfinished.
  openRowMenu('Homelab');
  const add = menuItem('Homelab', 'Add someone on foks.example.net…');
  assert.equal(inert(add), true);
  assert.equal(
    add.getAttribute('title'),
    'Memberships can’t be changed in an ad-hoc group.',
  );
  assert.equal(inert(menuItem('Homelab', 'Finish setup…')), false);
  // Copying the group ID is a local fact, so it applies throughout.
  assert.equal(inert(menuItem('Homelab', 'Copy group ID')), false);
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
  const unread = 'The roster could not be read. Refresh before making changes.';
  openRowMenu('Engineering');
  const add = menuItem('Engineering', 'Add someone on foks.acme-corp.com…');
  assert.equal(inert(add), true);
  assert.equal(add.getAttribute('title'), unread);
  // Admitting a group is refused for the same reason: the role that would
  // permit it is a roster fact, so an unread roster settles nothing about it.
  const admit = menuItem('Engineering', 'Add a group…');
  assert.equal(inert(admit), true);
  assert.equal(admit.getAttribute('title'), unread);
  // An unread roster is not evidence of sole ownership, so neither sentence
  // about who can remove you is offered.
  assert.equal(
    menuItem('Engineering', 'Leave…').getAttribute('title'),
    'Leaving a group is not available yet.',
  );
});

test('a server row whose access lapsed says so with a chip', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/lease.ts',
  )) as typeof import('../src/model/lease');
  await teams(() => {}, { snapshot: applyLease(await fixture(), 'lapsed') });
  const work = row('foks.acme-corp.com');
  assert.equal(stateChip(work), 'Check-in expired');
  // The caption still says only which account the row is.
  assert.equal(
    work.querySelector('.name small')?.textContent,
    'as vitalik · work',
  );
  const invite = [...work.querySelectorAll('button')].find(
    (button) => button.textContent === 'Invite someone…',
  );
  assert.ok(invite);
  assert.equal(invite.disabled, true);
  // The invitation names the server the invitee joins, so it says so.
  assert.equal(
    invite.getAttribute('title'),
    'Restore access to foks.acme-corp.com before inviting someone.',
  );
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
    created.getByRole('heading', { name: 'Create a group' }).textContent,
    'Create a group',
  );
  // Creating acts as one account on one server, and the sheet says which —
  // both in its subtitle and in the choice it is seeded to.
  assert.equal(
    document.querySelector('.sheet .hd small')?.textContent,
    'vitalik on foks.acme-corp.com',
  );
  assert.equal(seededAccount(), 'foks.acme-corp.com');
  // The button says the alias the server is asked to create, not the name.
  assert.ok(created.getByRole('button', { name: 'Create platform' }));
  ui.cleanup();

  // Another account in the address seeds the sheet to that one instead.
  await teams(() => {}, { store: 'acct:personal', scene: 'create' });
  assert.equal(
    document.querySelector('.sheet .hd small')?.textContent,
    'satoshi on foks.example.net',
  );
  assert.equal(seededAccount(), 'foks.example.net');
  ui.cleanup();

  const joining = await teams(() => {}, {
    store: 'acct:work',
    scene: 'join',
  });
  assert.ok(joining.getByRole('heading', { name: 'Join a group' }));
  assert.equal(
    document.querySelector('.sheet .hd small')?.textContent,
    'vitalik on foks.acme-corp.com',
  );
});

test('an account row invites as its own account, and the sheet can change it', async () => {
  const rendered = await teams();
  const work = row('foks.acme-corp.com');
  const invite = [...work.querySelectorAll('button')].find(
    (button) => button.textContent === 'Invite someone…',
  );
  assert.ok(invite);
  await ui.act(async () => {
    ui.fireEvent.click(invite);
  });
  assert.ok(rendered.getByRole('heading', { name: 'Invite someone' }));
  // The account is the first choice on the sheet, seeded to the row it was
  // opened from, and it is what the consequence line and the message say.
  const picker = document.querySelector(
    '[role="radiogroup"][aria-label="Invite as"]',
  );
  assert.ok(picker);
  assert.equal(
    picker.querySelector('[aria-checked="true"] .t b')?.textContent,
    'vitalik',
  );
  assert.match(
    document.querySelector('.band.info')?.textContent ?? '',
    /You are inviting as vitalik, on foks\.acme-corp\.com/,
  );
  assert.match(
    document.querySelector('.copybox .v')?.textContent ?? '',
    /enter foks\.acme-corp\.com/,
  );
  assert.match(
    document.querySelector('.copybox .v')?.textContent ?? '',
    /Ask me for the FOKS installer and install it/,
  );
  assert.doesNotMatch(document.body.textContent ?? '', /foks\.app\/download/);
  assert.match(
    document.body.textContent ?? '',
    /may require a signup code; get that code from the server administrator/,
  );

  // Choosing the other account rewrites the band, the group list and the
  // message: an invitation is to one server, and this is which.
  const other = [
    ...picker.querySelectorAll<HTMLButtonElement>('[role="radio"]'),
  ].find((node) => node.querySelector('.t b')?.textContent === 'satoshi');
  assert.ok(other);
  await ui.act(async () => {
    ui.fireEvent.click(other);
  });
  assert.match(
    document.querySelector('.band.info')?.textContent ?? '',
    /You are inviting as satoshi, on foks\.example\.net/,
  );
  const groups = document.querySelector(
    '[role="radiogroup"][aria-label="Which group you plan to add them to"]',
  );
  assert.ok(groups);
  assert.deepEqual(
    [...groups.querySelectorAll('[role="radio"] .t b')].map(
      (node) => node.textContent,
    ),
    ['No group yet', 'Household'],
  );
});

test('a group row invites as the account that holds the group, naming it', async () => {
  const rendered = await teams();
  openRowMenu('Engineering');
  await ui.act(async () => {
    ui.fireEvent.click(menuItem('Engineering', 'Invite someone…'));
  });
  assert.ok(rendered.getByRole('heading', { name: 'Invite someone' }));
  assert.equal(
    document.querySelector(
      '[role="radiogroup"][aria-label="Which group you plan to add them to"] [aria-checked="true"] .t b',
    )?.textContent,
    'Engineering',
  );
  // The message names the group, and says the one step that grants access.
  const message = document.querySelector('.copybox .v')?.textContent ?? '';
  assert.match(message, /I'd like to add you to Engineering on FOKS\./);
  assert.match(message, /FOKS does not use invite links/);
});

test('the per-account checks are folded into one row', async () => {
  const rendered = await teams();
  const disclosure = rendered.getByRole('button', {
    name: /Check other servers for groups/,
  });
  // Folded by default: discovery is an occasional per-server action.
  assert.equal(disclosure.getAttribute('aria-expanded'), 'false');
  const body = document.getElementById('teams-servers');
  assert.ok(body);
  assert.equal(body.hasAttribute('hidden'), true);
  // The head counts what it holds rather than listing it.
  assert.equal(disclosure.querySelector('.tail')?.textContent, '2 servers');
  await ui.act(async () => {
    ui.fireEvent.click(disclosure);
  });
  assert.equal(disclosure.getAttribute('aria-expanded'), 'true');
  assert.equal(body.hasAttribute('hidden'), false);
  assert.equal(rendered.getAllByText('Check for groups').length, 2);
});

test('a lapsed account opens the checks rather than hiding its state', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/lease.ts',
  )) as typeof import('../src/model/lease');
  const rendered = await teams(() => {}, {
    snapshot: applyLease(await fixture(), 'lapsed'),
  });
  // The row is the only report of that state, so it is not folded away.
  assert.equal(
    rendered
      .getByRole('button', { name: /Check other servers for groups/ })
      .getAttribute('aria-expanded'),
    'true',
  );
  assert.equal(
    document.getElementById('teams-servers')?.hasAttribute('hidden'),
    false,
  );
});
