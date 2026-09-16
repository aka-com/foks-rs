/**
 * The group page's Members tab.
 *
 * The roster is drawn as three sub-sections — people, machines and the groups
 * admitted from other servers — one line of role each: the visibility band
 * rides inside the role chip and the party generation is not shown. What a row
 * cannot do stays in its menu, inert and still reachable, with the reason in
 * its title, and an alert that carries one action puts it at the right end of
 * the alert.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';
import type { Bridge, PendingOperation } from '../src/bridge';
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

async function fixture(): Promise<AgentSnapshot> {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  return FIXTURE;
}

async function group(
  snapshot: AgentSnapshot,
  overrides: Partial<Bridge> = {},
  ref = 'team:eng',
  tab: 'people' | 'settings' = 'people',
) {
  const { GroupSettingsScreen } = (await vite.ssrLoadModule(
    '/src/screens/groups-screen.tsx',
  )) as typeof import('../src/screens/groups-screen');
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
        children: createElement(GroupSettingsScreen, {
          snapshot,
          bridge: { ...mockBridge(snapshot), ...overrides },
          location: { kind: 'group-settings', ref, tab },
          onNavigate: () => {},
          onApplied: async () => {},
          onError: (error: unknown) => {
            throw error;
          },
          onMutationError: async () => {},
        }),
      }),
    }),
  );
  await ui.act(async () => {});
  return rendered;
}

/** The member row a name is drawn in. */
function memberRow(name: string): HTMLElement {
  const node = [...document.querySelectorAll<HTMLElement>('.rt .prow')].find(
    (candidate) =>
      candidate.querySelector('.who2 .t b span')?.textContent === name,
  );
  assert.ok(node, `no member row for ${name}`);
  return node;
}

/**
 * An item of the open menu. Items that do not apply keep their place in the
 * menu, so they are found by label rather than by an interactive role, and an
 * item may carry a glyph before its label.
 */
function menuItem(label: string): HTMLButtonElement {
  const node = [
    ...document.querySelectorAll<HTMLButtonElement>('.menu button'),
  ].find((candidate) => (candidate.textContent ?? '').trim().endsWith(label));
  assert.ok(node, `no menu item labelled ${label}`);
  return node;
}

/** Whether an item is inert: present, focusable, and doing nothing. */
function inert(node: HTMLButtonElement): boolean {
  return node.getAttribute('aria-disabled') === 'true';
}

/** The label of the sub-section a row belongs to, with its count. */
function sectionLabels(): string[] {
  return [...document.querySelectorAll('.sec')].map(
    (node) => node.textContent ?? '',
  );
}

test('the roster is split into people, machines and admitted groups', async () => {
  await group(await fixture());
  assert.deepEqual(sectionLabels(), [
    'People— 4 people',
    'Machines— 1 connected',
    'Groups on other servers',
  ]);
  // A machine is an ordinary party; its line says only what kind it is.
  assert.equal(
    memberRow('deploy-bot').querySelector('small')?.textContent,
    'machine',
  );
  assert.equal(
    memberRow('sam.ortiz').querySelector('small')?.textContent,
    'person',
  );
  // The generation is not drawn on a row; only the raw response still holds it.
  const rows = [...document.querySelectorAll('.rt .prow')]
    .map((node) => node.textContent ?? '')
    .join(' ');
  assert.equal(rows.includes('generation'), false);
});

test('Members exposes invitation creation, requests and approval recovery for this group', async () => {
  const calls: { profile: string; account: string; action: unknown }[] = [];
  const snapshot = await fixture();
  const store = snapshot.stores.find((entry) => entry.id === 'team:eng');
  assert.ok(store?.kind === 'team');
  const r = await group(snapshot, {
    invitation: async (profile, account, action) => {
      calls.push({ profile, account, action });
      return action.action === 'inbox'
        ? {
            rows: [
              {
                request_id: '1'.repeat(32),
                username: 'new-member',
                verified: true,
              },
            ],
          }
        : { state: 'complete' };
    },
  });
  await ui.act(async () => {});
  calls.length = 0;
  ui.fireEvent.click(
    r.getByRole('button', { name: 'Invitations and requests' }),
  );
  for (const label of [
    'Create invitation',
    'Refresh requests',
    'Approve',
    'Reject',
    'Show pending approvals',
    'Show pending operations',
  ]) {
    ui.fireEvent.click(r.getByRole('button', { name: label }));
    await ui.act(async () => {});
  }
  assert.deepEqual(
    calls.map((call) => (call.action as { action: string }).action),
    [
      // Confirmed or pending mutations refresh the visible recovery count.
      'create',
      'list',
      'pending-approvals',
      'inbox',
      'approve',
      'list',
      'pending-approvals',
      'reject',
      'list',
      'pending-approvals',
      // These explicit read-only actions do not invalidate it again.
      'pending-approvals',
      'list',
    ],
  );
  assert.ok(
    calls.every(
      (call) => call.profile === store.server && call.account === store.account,
    ),
  );
  assert.deepEqual(calls[0].action, {
    action: 'create',
    team_alias: store.alias,
  });
});

test('Members does not expose invitation administration to an ordinary member', async () => {
  const snapshot = await fixture();
  await group({
    ...snapshot,
    parties: snapshot.parties.map((party) =>
      party.store === 'team:eng' && party.label === 'you'
        ? { ...party, destination_role: 'member' }
        : party,
    ),
  });
  assert.equal(ui.screen.queryByText('Invitations and requests'), null);
});

test('a role is one chip, with the visibility band inside it', async () => {
  await group(await fixture());
  assert.equal(
    memberRow('dana.okafor').querySelector('.rolecell .chip')?.textContent,
    'Member (0)',
  );
  // Owner and Admin have no band, so their chip carries none.
  assert.equal(
    memberRow('sam.ortiz').querySelector('.rolecell .chip')?.textContent,
    'Owner',
  );
  assert.equal(
    memberRow('vitalik').querySelector('.chip.you')?.textContent,
    'you',
  );
  // The role cell holds the chip alone: no second line under it.
  assert.equal(memberRow('dana.okafor').querySelector('.rolecell small'), null);
});

test('a row menu carries actions only, inert with the reason in the title', async () => {
  const rendered = await group(await fixture());
  // An Admin cannot target the Owner: the items stay, inert, with why.
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Actions for sam.ortiz' }),
  );
  const blocked = menuItem('Lower role…');
  assert.equal(inert(blocked), true);
  // `aria-disabled`, not `disabled`: the keyboard still reaches the item, so
  // the reason can be read out rather than only hovered.
  assert.equal(blocked.hasAttribute('disabled'), false);
  assert.equal(blocked.tabIndex, -1);
  assert.equal(
    blocked.getAttribute('title'),
    'An Admin cannot change another Admin or the Owner.',
  );
  const descriptionId = blocked.getAttribute('aria-describedby');
  assert.ok(descriptionId);
  assert.equal(
    document.getElementById(descriptionId)?.textContent,
    'An Admin cannot change another Admin or the Owner.',
  );
  // The menu says nothing beyond its actions.
  const menu = blocked.closest('.menu');
  assert.ok(menu);
  assert.equal(menu.querySelector('p'), null);
  ui.fireEvent.keyDown(document, { key: 'Escape' });

  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Actions for dana.okafor' }),
  );
  assert.equal(inert(menuItem('Lower role…')), false);
});

test('an admitted group is one row: role, admission state and its own actions', async () => {
  const rendered = await group(await fixture());
  const row = memberRow('homelab');
  assert.equal(
    row.querySelector('.kico.round')?.getAttribute('aria-hidden'),
    'true',
  );
  const chips = [...row.querySelectorAll('.chip')].map(
    (chip) => chip.textContent,
  );
  assert.deepEqual(chips, ['Member (0)', 'Inactive']);
  assert.ok(rendered.getByRole('button', { name: 'Restore access' }));
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Actions for homelab' }),
  );
  const lower = menuItem('Lower role…');
  assert.equal(inert(lower), true);
  assert.match(
    lower.getAttribute('title') ?? '',
    /managed on their own server/,
  );
  // The admission itself can be removed, once it is active again.
  const remove = menuItem('Remove admission…');
  assert.equal(inert(remove), true);
  assert.equal(
    remove.getAttribute('title'),
    'Restore access before removing this admission.',
  );
});

test('a single-action alert puts its action at the right end of the alert', async () => {
  const snapshot = await fixture();
  const rendered = await group({
    ...snapshot,
    groupDetailFailures: [
      {
        store: 'team:eng',
        source: 'roster',
        code: 'unavailable',
        message: 'The group roster could not be read from the agent.',
        retryable: true,
      },
    ],
  });
  const alert = document.querySelector('.band');
  assert.ok(alert);
  // The label is the sentence's bold lead-in, not a heading of the page.
  assert.equal(alert.querySelector('b')?.textContent, 'Roster unavailable');
  assert.equal(alert.querySelector('h2'), null);
  // A band the page always draws does not announce itself on every mount; it
  // is a group the reader can reach by its label.
  assert.equal(alert.getAttribute('role'), 'group');
  assert.equal(alert.getAttribute('aria-label'), 'Roster unavailable');
  // The action sits in the band's own action cell, which centres it at the
  // right end rather than inline in the sentence.
  const refresh = rendered.getByRole('button', { name: 'Refresh' });
  assert.equal(refresh.closest('.band .a') !== null, true);
  assert.equal(alert.lastElementChild?.className, 'a');
  // Federation still loaded, so its section is drawn as usual.
  assert.ok(sectionLabels().includes('Groups on other servers'));
  // The add-actions belong to the rows that failed to load, so they go with
  // them: the only one left is what admits another group.
  const actions = [...document.querySelectorAll('.roster-actions')];
  assert.equal(actions.length, 1);
  assert.equal(
    (actions[0].querySelector('button')?.textContent ?? '').trim(),
    'Add a group…',
  );
  assert.equal(
    rendered.queryByRole('button', { name: 'Invite to group…' }),
    null,
  );
});

test('a federation failure replaces its rows and keeps its own Refresh', async () => {
  const snapshot = await fixture();
  const rendered = await group({
    ...snapshot,
    groupDetailFailures: [
      {
        store: 'team:eng',
        source: 'federation',
        code: 'unavailable',
        message: 'The admitted groups could not be read from the agent.',
        retryable: true,
      },
    ],
  });
  const refresh = rendered.getByRole('button', { name: 'Refresh' });
  const alert = refresh.closest('.band');
  assert.ok(alert);
  assert.equal(alert.querySelector('b')?.textContent, 'Federation unavailable');
  assert.equal(refresh.closest('.band .a') !== null, true);
  // The roster is unaffected, so its rows stay.
  assert.ok(memberRow('sam.ortiz'));
});

test('a pending membership change offers Resume at the right end of its alert', async () => {
  const snapshot = await fixture();
  const pending: PendingOperation[] = [
    {
      kind: 'team-member-addition',
      alias: 'engineering',
      target: 'jules.park',
    },
  ];
  const rendered = await group(snapshot, {
    listPendingOperations: async () => pending,
  });
  const resume = rendered.getByRole('button', {
    name: 'Resume adding jules.park',
  });
  const alert = resume.closest('.band');
  assert.ok(alert);
  assert.equal(
    alert.querySelector('b')?.textContent,
    'Finish a pending membership change',
  );
  assert.equal(resume.closest('.band .a') !== null, true);
  // This band is mounted by what the reader did — a change that stopped
  // partway, read back after the action — so it is announced.
  assert.equal(alert.getAttribute('role'), 'status');
});

test('the Members count counts an admitted group once', async () => {
  const snapshot = await fixture();
  await group(snapshot);
  const members = [...document.querySelectorAll('[role="tab"]')].find((tab) =>
    tab.textContent?.startsWith('Members'),
  );
  assert.ok(members);
  // Four people, one machine and one admitted group — reported both as a
  // roster party and as a federation entry, and counted once.
  assert.equal(members.querySelector('.n')?.textContent, '6');
});

test('a roster party with no admission record is listed, not dropped', async () => {
  const snapshot = await fixture();
  // The admission record is missing, so nothing matches the roster's group.
  await group({ ...snapshot, federation: [] });
  const row = memberRow('homelab');
  assert.equal(
    row.querySelector('.kico.round')?.getAttribute('aria-hidden'),
    'true',
  );
  assert.equal(row.querySelector('small')?.textContent, 'on foks.example.net');
  const chips = [...row.querySelectorAll('.chip')].map(
    (chip) => chip.textContent,
  );
  assert.deepEqual(chips, ['Member (0)', 'No admission record']);
  // It is still a member of this group, so it is still counted.
  const members = [...document.querySelectorAll('[role="tab"]')].find((tab) =>
    tab.textContent?.startsWith('Members'),
  );
  assert.equal(members?.querySelector('.n')?.textContent, '6');
});

test('an ambiguous admission record is one row, counted once', async () => {
  const snapshot = await fixture();
  const [entry] = snapshot.federation;
  // Two records on this Mac name the same remote group on the same host, so
  // neither of them can be said to be the one the roster means.
  await group({
    ...snapshot,
    federation: [
      entry,
      {
        ...entry,
        remote_team_alias: 'homelab (stale)',
        operation_id_hex: '3ab91c40',
      },
    ],
  });
  const row = memberRow('homelab');
  assert.deepEqual(
    [...row.querySelectorAll('.chip')].map((chip) => chip.textContent),
    ['Member (0)', 'Ambiguous admission'],
  );
  // The records it matches are left to that one row rather than listed beside
  // it, so the group appears once.
  assert.equal(document.querySelectorAll('.rt.fed .prow').length, 1);
  const members = [...document.querySelectorAll('[role="tab"]')].find((tab) =>
    tab.textContent?.startsWith('Members'),
  );
  assert.equal(members?.querySelector('.n')?.textContent, '6');
});

test('the group sections are a tablist the arrow keys walk', async () => {
  const rendered = await group(await fixture());
  const tabs = document.querySelector('[role="tablist"]');
  assert.ok(tabs);
  assert.equal(tabs.getAttribute('aria-label'), 'Group sections');
  const strip = [...tabs.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
  // Four tabs, in the order the page's addresses name them.
  assert.deepEqual(
    strip.map((tab) => tab.getAttribute('data-tab')),
    ['people', 'channels', 'files', 'settings'],
  );
  const [members, channels] = strip;
  const settings = strip[3];
  // The open tab is the one in the page's tab order; the arrows reach the rest.
  assert.equal(members.getAttribute('aria-selected'), 'true');
  assert.equal(members.tabIndex, 0);
  assert.equal(settings.tabIndex, -1);
  // The strip names the panel it opened, and the panel names the tab back.
  const panel = document.querySelector('[role="tabpanel"]');
  assert.ok(panel);
  assert.equal(members.getAttribute('aria-controls'), panel.id);
  assert.equal(panel.getAttribute('aria-labelledby'), members.id);
  // Only the open panel is mounted, so a closed tab points at nothing.
  assert.equal(settings.hasAttribute('aria-controls'), false);
  await ui.act(async () => {
    members.focus();
    ui.fireEvent.keyDown(tabs, { key: 'ArrowRight' });
  });
  // Selection follows focus: the arrow both moves and opens.
  assert.equal(channels.getAttribute('aria-selected'), 'true');
  assert.equal(document.activeElement, channels);
  await ui.act(async () => {
    ui.fireEvent.keyDown(tabs, { key: 'End' });
  });
  assert.equal(settings.getAttribute('aria-selected'), 'true');
  assert.ok(rendered.getByText('About this group'));
  assert.equal(
    document
      .querySelector('[role="tabpanel"]')
      ?.getAttribute('aria-labelledby'),
    settings.id,
  );
});

test('a member cannot act on their own row', async () => {
  const rendered = await group(await fixture());
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Actions for vitalik' }),
  );
  const remove = menuItem('Remove…');
  assert.equal(inert(remove), true);
  assert.equal(
    remove.getAttribute('title'),
    'You cannot change your own role or remove your own account.',
  );
});

test('each action follows the rows it adds to', async () => {
  const rendered = await group(await fixture());
  const actions = [...document.querySelectorAll('.roster-actions')];
  assert.equal(actions.length, 2);
  // People and machines first, then what adds to them.
  assert.deepEqual(
    [...actions[0].querySelectorAll('button')].map((node) =>
      (node.textContent ?? '').trim(),
    ),
    ['Add someone on Acme…', 'Invite to group…'],
  );
  // Then the admitted groups, then what admits another one.
  const federation = document.querySelector('.rt.fed');
  assert.ok(federation);
  assert.equal(
    federation.compareDocumentPosition(actions[1]) &
      Node.DOCUMENT_POSITION_FOLLOWING,
    Node.DOCUMENT_POSITION_FOLLOWING,
  );
  assert.equal(
    (actions[1].querySelector('button')?.textContent ?? '').trim(),
    'Add a group…',
  );
  // And the sheet it opens says what it will do, in its own words, naming
  // the group it was given rather than "group".
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Add a group…' }));
  });
  assert.ok(rendered.getByRole('button', { name: 'Add household' }));
  assert.equal(
    document.querySelector('.sheet .vis-value')?.textContent,
    'Visibility 0',
  );
});

test('an active ad-hoc share says its membership is fixed', async () => {
  const snapshot = await fixture();
  // The fixture's only ad-hoc group never finished setup, so the active case
  // is built here: the same two parties, fixed at creation.
  const share = {
    id: 'team:offsite',
    kind: 'team' as const,
    name: 'Offsite',
    alias: 'offsite',
    server: 'personal',
    account: 'personal',
    active: true,
    team_kind: 'adhoc' as const,
    team_id_hex:
      '0399c2aa07b45e18d0c73a9f2e5b6417ac8d0192f3e4b5c6d7089a1b2c3d4e5f31',
  };
  await group(
    {
      ...snapshot,
      stores: [...snapshot.stores, share],
      storeInventory: [
        ...snapshot.storeInventory,
        { store: share.id, status: 'available' as const, restrictions: [] },
      ],
      parties: [
        ...snapshot.parties,
        ...snapshot.parties
          .filter((party) => party.store === 'team:household')
          .map((party) => ({
            ...party,
            store: share.id,
            party_id_hex: `9${party.party_id_hex.slice(1)}`,
          })),
      ],
    },
    {},
    share.id,
  );
  const band = document.querySelector('.band.info');
  assert.ok(band);
  assert.equal(band.querySelector('b')?.textContent, 'Ad-hoc group');
  // The page always draws this one, so it names itself rather than speaking.
  assert.equal(band.getAttribute('role'), 'group');
  assert.match(band.textContent ?? '', /Memberships can’t be changed\./);
  // A fixed membership has nothing to add to, so no action is offered.
  assert.equal(document.querySelector('.roster-actions'), null);
});

test('the Settings tab states the name, the join policy and why leaving is not offered', async () => {
  const rendered = await group(await fixture(), {}, 'team:eng', 'settings');
  const rows = [...document.querySelectorAll('.inset .fr')];
  const nameRow = rows.find(
    (row) => row.querySelector('.k')?.textContent === 'Name',
  );
  assert.ok(nameRow);
  assert.equal(
    nameRow.querySelector('.hint')?.textContent,
    'A name is fixed at creation.',
  );
  const policy = rows.find(
    (row) => row.querySelector('.k')?.textContent === 'Join policy',
  );
  assert.ok(policy);
  assert.match(policy.textContent ?? '', /Invite only/);
  assert.equal(policy.querySelector('button'), null);
  // The role reads the way every other role on this page reads.
  const account = rows.find(
    (row) => row.querySelector('.k')?.textContent === 'Your account',
  );
  assert.equal(account?.querySelector('.v')?.textContent, 'vitalik · Admin');
  assert.equal(rendered.queryByRole('button', { name: 'Leave…' }), null);
  assert.equal(rendered.queryByRole('button', { name: 'Delete…' }), null);
});

test('Members invite action opens the group invitation workflow', async () => {
  const calls: unknown[] = [];
  const r = await group(await fixture(), {
    invitation: async (_profile, _account, action) => {
      calls.push(action);
      return { state: 'complete' };
    },
  });
  await ui.act(async () => {});
  calls.length = 0;
  ui.fireEvent.click(r.getByRole('button', { name: 'Invite to group…' }));
  const dialog = r.getByRole('dialog');
  ui.fireEvent.click(
    ui.within(dialog).getByRole('button', { name: 'Create invitation' }),
  );
  await ui.act(async () => {});
  assert.deepEqual(calls, [{ action: 'create', team_alias: 'engineering' }]);
});

test('an invitation prepared after unmount is recovered from the group banner', async () => {
  const snapshot = await fixture();
  const team = snapshot.stores.find((entry) => entry.id === 'team:eng');
  assert.ok(team?.kind === 'team');
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let prepared = false;
  const row = {
    operation_id: 'a'.repeat(32),
    team_id: team.team_id_hex,
    state: 'prepared',
  };
  const invitation: Bridge['invitation'] = async (
    _profile,
    _account,
    action,
  ) => {
    if (action.action === 'list') return prepared ? [row] : [];
    if (action.action === 'pending-approvals') return [];
    if (action.action === 'create') {
      await gate;
      prepared = true;
      return row;
    }
    return row;
  };
  const first = await group(snapshot, { invitation });
  ui.fireEvent.click(first.getByRole('button', { name: 'Invite to group…' }));
  ui.fireEvent.click(
    ui
      .within(first.getByRole('dialog'))
      .getByRole('button', { name: 'Create invitation' }),
  );
  first.unmount();
  await ui.act(async () => {
    release();
  });
  const returned = await group(snapshot, { invitation });
  ui.fireEvent.click(
    await returned.findByRole('button', { name: 'Review invitations' }),
  );
  const dialog = returned.getByRole('dialog');
  await ui.within(dialog).findByText('prepared');
  assert.ok(ui.within(dialog).getByRole('button', { name: 'Submit' }));
});
