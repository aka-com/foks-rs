/**
 * The group page's Channels and Files tabs, the sheet that adds to a group,
 * and the page a group whose setup never finished shows instead of any tab.
 *
 * Channels lists the inbox data for this group and opens each row in Chat.
 * Files links to the group vault and displays the catalog item count.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';
import type { Bridge } from '../src/bridge';
import type { GroupSettingsTab, Location } from '../src/location';
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

/**
 * The group page inside the shell's chat inbox provider, which is where it
 * lives in the app: the Channels tab reads the same service the rail does.
 */
async function group(
  ref: string,
  tab: GroupSettingsTab,
  options: {
    snapshot?: AgentSnapshot;
    onNavigate?: (to: Location) => void;
    /** The shell's clock, for a group whose access lapses while it is open. */
    accessNow?: () => number;
    /** One command answered differently, to reach a refusal the mock never gives. */
    patchBridge?: (bridge: Bridge) => Bridge;
  } = {},
) {
  const { GroupSettingsScreen } = (await vite.ssrLoadModule(
    '/src/screens/groups-screen.tsx',
  )) as typeof import('../src/screens/groups-screen');
  const { ChatInboxProvider } = (await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
  )) as typeof import('../src/chat/inbox-provider');
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
  const base = mockBridge(snapshot);
  const bridge = options.patchBridge ? options.patchBridge(base) : base;
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(ChatInboxProvider, {
          bridge,
          snapshot,
          children: createElement(GroupSettingsScreen, {
            snapshot,
            bridge,
            location: { kind: 'group-settings', ref, tab },
            ...(options.accessNow ? { accessNow: options.accessNow } : {}),
            onNavigate: options.onNavigate ?? (() => {}),
            onApplied: async () => {},
            onError: (error: unknown) => {
              throw error;
            },
            onMutationError: async () => {},
          }),
        }),
      }),
    }),
  );
  await ui.act(async () => {});
  return rendered;
}

/** The channel rows, by the `#name` each draws. */
function channelRows(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('.roster .rt.bare .prow')];
}

test('the Channels tab says why a group on a chatless server has none', async () => {
  // Chat is not enabled for Engineering's server, Acme.
  const rendered = await group('team:eng', 'channels');
  const band = document.querySelector('.band.info');
  assert.ok(band);
  assert.equal(band.querySelector('b')?.textContent, 'No channels here');
  assert.match(band.textContent ?? '', /Chat is not enabled on Acme/);
  // Nothing is offered that the server could not take.
  assert.equal(rendered.queryByRole('button', { name: 'Add channel' }), null);
  // The tab itself carries no count, because there is no list to count.
  const tab = [...document.querySelectorAll('[role="tab"]')].find(
    (node) => node.getAttribute('data-tab') === 'channels',
  );
  assert.ok(tab);
  assert.equal(tab.querySelector('.n'), null);
});

test('the Channels tab reads the clock afresh, so a lapse closes it', async () => {
  // A check-in that outlasts the real clock, so only the tab's own clock can
  // lapse it: the page's takeover is not what answers here.
  const base = await fixture();
  const snapshot: AgentSnapshot = {
    ...base,
    servers: base.servers.map((server) =>
      server.id === 'personal'
        ? {
            ...server,
            compatibility: { status: 'required' as const, expiresAt: 4e9 },
          }
        : server,
    ),
  };
  let now = 1e9;
  const rendered = await group('team:household', 'channels', {
    snapshot,
    accessNow: () => now,
  });
  await ui.waitFor(() => assert.ok(channelRows().length));
  // The same page, a moment later: the check-in has expired.
  now = 5e9;
  const tab = (id: string): HTMLElement => {
    const node = [
      ...document.querySelectorAll<HTMLElement>('[role="tab"]'),
    ].find((candidate) => candidate.getAttribute('data-tab') === id);
    assert.ok(node);
    return node;
  };
  await ui.act(async () => {
    ui.fireEvent.click(tab('files'));
  });
  await ui.act(async () => {
    ui.fireEvent.click(tab('channels'));
  });
  assert.equal(channelRows().length, 0);
  const band = document.querySelector('.roster .band');
  assert.ok(band);
  // The condition, in the words the takeover uses for it — never the roster
  // summary a store's description falls back to.
  assert.equal(band.querySelector('b')?.textContent, 'Check-in expired');
  assert.match(
    band.textContent ?? '',
    /The session for Personal server has expired\./,
  );
  assert.equal(rendered.queryByRole('button', { name: 'Add channel' }), null);
});

test('a channel list that arrived with a problem keeps its rows', async () => {
  await group('team:household', 'channels', {
    patchBridge: (base) => ({
      ...base,
      chat: async (id, action, view) => {
        const reply = await base.chat(id, action, view);
        if (id === 'team:household' && reply.result.kind === 'inbox')
          reply.result.read_retry_pending = true;
        return reply;
      },
    }),
  });
  await ui.waitFor(() => assert.ok(channelRows().length));
  // What the synchronization could not finish is a note over the rows it did
  // bring, not a failure in place of them.
  const band = document.querySelector('.roster .band');
  assert.ok(band);
  assert.equal(
    band.querySelector('b')?.textContent,
    'Channels may be out of date',
  );
  assert.match(band.textContent ?? '', /Read status will retry\./);
});

test('the Channels tab lists the group’s channels and opens one in Chat', async () => {
  const journal: Location[] = [];
  // Household lives on Personal server, which offers chat.
  await group('team:household', 'channels', {
    onNavigate: (to) => journal.push(to),
  });
  await ui.waitFor(() => assert.ok(channelRows().length));
  const rows = channelRows();
  assert.equal(rows.length, 1);
  const general = rows[0];
  assert.equal(
    general.querySelector('.who2 .t b span')?.textContent,
    '#general',
  );
  // The channel's own description, else what it says about who can take part.
  assert.equal(
    general.querySelector('.who2 .t small')?.textContent,
    'A place for the whole team.',
  );
  const open = general.querySelector<HTMLButtonElement>(
    'button[aria-label="Open #general in Chat"]',
  );
  assert.ok(open);
  await ui.act(async () => {
    ui.fireEvent.click(open);
  });
  const opened = journal.at(-1);
  assert.equal(opened?.kind, 'chat');
  assert.equal(
    opened?.kind === 'chat' ? opened.ref : undefined,
    'team:household',
  );
  assert.ok(opened?.kind === 'chat' && opened.channel);
  // The tab strip counts the same list the rows came from.
  const tab = [...document.querySelectorAll('[role="tab"]')].find(
    (node) => node.getAttribute('data-tab') === 'channels',
  );
  assert.equal(tab?.querySelector('.n')?.textContent, '1');
});

test('Add channel opens the New channel step scoped to this group', async () => {
  const rendered = await group('team:household', 'channels');
  await ui.waitFor(() => assert.ok(channelRows().length));
  const add = rendered.getByRole('button', { name: 'Add channel' });
  assert.equal(add.hasAttribute('disabled'), false);
  await ui.act(async () => {
    ui.fireEvent.click(add);
  });
  // The sheet opens on the step it was asked for: the create form for this
  // group, not the channel picker and not the cross-team one.
  assert.ok(rendered.getByRole('heading', { name: 'New channel' }));
  assert.ok(rendered.getByLabelText('Channel name'));
  assert.match(
    document.querySelector('.sheet .hd small')?.textContent ?? '',
    /^Household · /,
  );
  // This flow has no preceding team step, so the left button cancels instead
  // of opening the team picker.
  assert.equal(rendered.queryByRole('button', { name: 'Back' }), null);
  assert.equal(rendered.queryByRole('radio', { name: /^Engineering/ }), null);
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Cancel' }));
  });
  assert.equal(rendered.queryByRole('dialog'), null);
});

test('a channel created from the group page opens it in Chat', async () => {
  const journal: Location[] = [];
  const rendered = await group('team:household', 'channels', {
    onNavigate: (to) => journal.push(to),
  });
  await ui.waitFor(() => assert.ok(channelRows().length));
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Add channel' }));
  });
  await ui.act(async () => {
    ui.fireEvent.change(rendered.getByLabelText('Channel name'), {
      target: { value: 'garden' },
    });
  });
  const create = rendered.getByRole('button', { name: 'Create channel' });
  assert.equal(create.hasAttribute('disabled'), false);
  await ui.act(async () => {
    ui.fireEvent.click(create);
  });
  // The submission is the agent's own preparation, attempted and settled, so
  // the sheet closes on the channel it made and the page hands it to Chat.
  await ui.waitFor(() => assert.equal(rendered.queryByRole('dialog'), null));
  const opened = journal.at(-1);
  assert.equal(opened?.kind, 'chat');
  assert.equal(
    opened?.kind === 'chat' ? opened.ref : undefined,
    'team:household',
  );
  assert.ok(opened?.kind === 'chat' && opened.channel);
});

test('the Files tab links to the group vault and displays its item count', async () => {
  const journal: Location[] = [];
  const rendered = await group('team:eng', 'files', {
    onNavigate: (to) => journal.push(to),
  });
  const row = document.querySelector('.roster .inset .fr');
  assert.ok(row);
  assert.equal(
    row.querySelector('.t b')?.textContent,
    'View Engineering’s items in Files',
  );
  // The count is the catalog's, said once.
  assert.match(row.querySelector('.t small')?.textContent ?? '', /^4 items — /);
  const tab = [...document.querySelectorAll('[role="tab"]')].find(
    (node) => node.getAttribute('data-tab') === 'files',
  );
  assert.equal(tab?.querySelector('.n')?.textContent, '4');
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Open in Files' }));
  });
  assert.deepEqual(journal.at(-1), { kind: 'store', ref: 'team:eng' });
  // This tab only links to Files; it does not render item rows.
  assert.equal(document.querySelector('.rt.bare'), null);
});

test('the add sheet switches between a person and another server’s group', async () => {
  const rendered = await group('team:eng', 'people');
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', {
        name: 'Add someone on Acme…',
      }),
    );
  });
  const seg = document.querySelector(
    '[role="group"][aria-label="What to add"]',
  );
  assert.ok(seg);
  const [person, remote] = [
    ...seg.querySelectorAll<HTMLButtonElement>('button'),
  ];
  assert.equal(person.getAttribute('aria-pressed'), 'true');
  assert.equal(remote.getAttribute('aria-pressed'), 'false');

  // The person half: the username, the server it is fixed to, and one role
  // card per role, with the one this account cannot grant kept and explained.
  const roles = document.querySelector(
    '[role="radiogroup"][aria-label="Role in Engineering"]',
  );
  assert.ok(roles);
  const cards = [
    ...roles.querySelectorAll<HTMLButtonElement>('[role="radio"]'),
  ];
  assert.deepEqual(
    cards.map((card) => card.querySelector('.t b')?.textContent),
    ['Owner', 'Admin', 'Member'],
  );
  // This account is an Admin of Engineering, so Owner is drawn with why.
  assert.equal(cards[0].className.includes('off'), true);
  assert.equal(
    cards[0].querySelector('.t small')?.textContent,
    'Only an Owner can add another Owner.',
  );
  assert.equal(cards[2].getAttribute('aria-checked'), 'true');
  // The band the Member role reads at is the one the steppers set.
  assert.equal(
    document.querySelector('.sheet .vis-value')?.textContent,
    'Visibility 0',
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Lower the visibility band' }),
    );
  });
  assert.equal(
    document.querySelector('.sheet .vis-value')?.textContent,
    'Visibility -1',
  );

  // The group half: a picker over the remote groups this Mac holds, and the
  // sentence that says what admitting one does.
  await ui.act(async () => {
    ui.fireEvent.click(remote);
  });
  assert.ok(
    rendered.getByRole('heading', { name: 'Add a group to Engineering' }),
  );
  const picker = document.querySelector(
    '[role="radiogroup"][aria-label="Group"]',
  );
  assert.ok(picker);
  assert.deepEqual(
    [...picker.querySelectorAll('[role="radio"] .t b')].map(
      (node) => node.textContent,
    ),
    ['household'],
  );
  const band = document.querySelector('.sheet .band.info');
  assert.equal(band?.querySelector('b')?.textContent, 'How admission works');
  assert.match(
    band?.textContent ?? '',
    /only the whole admission can be removed/,
  );
  // The band the person half was given belongs to that half: this one starts
  // where a new admission starts, not where the other answer was left.
  assert.equal(
    document.querySelector('.sheet .vis-value')?.textContent,
    'Visibility 0',
  );
  // Its steppers say what they change, as the person half's do.
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Raise the visibility band' }),
    );
  });
  assert.equal(
    document.querySelector('.sheet .vis-value')?.textContent,
    'Visibility 1',
  );
  // And switching back does not inherit the admission's band either. The
  // switch is re-read: each half draws its own, so the earlier node is gone.
  await ui.act(async () => {
    const control = document.querySelector(
      '[role="group"][aria-label="What to add"]',
    );
    assert.ok(control);
    ui.fireEvent.click(
      [...control.querySelectorAll<HTMLButtonElement>('button')][0],
    );
  });
  assert.equal(
    document.querySelector('.sheet .vis-value')?.textContent,
    'Visibility 0',
  );
});

test('the add sheet clears the agent’s refusal when the username changes', async () => {
  const rendered = await group('team:eng', 'people', {
    patchBridge: (bridge) => ({
      ...bridge,
      addGroupMember: async () => {
        throw new Error('No user named nobody.one on this server.');
      },
    }),
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', {
        name: 'Add someone on Acme…',
      }),
    );
  });
  const field = rendered.getByLabelText('Username');
  await ui.act(async () => {
    ui.fireEvent.change(field, { target: { value: 'nobody.one' } });
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Add nobody.one' }),
    );
  });
  assert.equal(
    document.querySelector('.sheet [role="alert"]')?.textContent,
    'No user named nobody.one on this server.',
  );
  // The refusal was of the username that was sent, so a different one is not
  // refused yet.
  await ui.act(async () => {
    ui.fireEvent.change(field, { target: { value: 'nobody.two' } });
  });
  assert.equal(document.querySelector('.sheet [role="alert"]'), null);
});

test('add member dialog rejects usernames already present in roster', async () => {
  const rendered = await group('team:eng', 'people');
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', {
        name: 'Add someone on Acme…',
      }),
    );
  });
  await ui.act(async () => {
    ui.fireEvent.change(rendered.getByLabelText('Username'), {
      target: { value: 'dana.okafor' },
    });
  });
  const refusal = document.querySelector('.sheet [role="alert"]');
  assert.ok(refusal);
  assert.equal(
    refusal.textContent,
    'dana.okafor is already a member of Engineering. Change their role from the Members list instead.',
  );
  assert.equal(
    rendered
      .getByRole('button', { name: 'Add dana.okafor' })
      .hasAttribute('disabled'),
    true,
  );
});

test('a group whose setup never finished has no tabs, and two ways out', async () => {
  // Homelab was created but its key setup never finished on this Mac.
  const rendered = await group('team:homelab', 'people');
  assert.equal(document.querySelector('[role="tablist"]'), null);
  const band = document.querySelector('.band');
  assert.ok(band);
  assert.equal(band.querySelector('b')?.textContent, 'Setup incomplete');
  // One sentence for the condition, the store page's own: the page and the
  // takeover cannot describe it differently.
  assert.match(
    band.textContent ?? '',
    /Homelab was created on Personal server, but key setup is incomplete on this Mac\./,
  );
  assert.equal(band.textContent?.includes('Nothing runs while FOKS'), false);
  assert.ok(
    band.contains(rendered.getByRole('button', { name: 'Finish setup' })),
  );
  assert.equal(
    rendered.queryByRole('button', { name: 'Remove and rotate keys…' }),
    null,
  );
  // What the page can still say about the group it cannot open.
  const labels = [...document.querySelectorAll('.roster .inset .fr .k')].map(
    (node) => node.textContent,
  );
  assert.deepEqual(labels, ['Account', 'Group ID']);
});

test('Finish setup resumes the existing incomplete group', async () => {
  const resumed: string[] = [];
  const rendered = await group('team:homelab', 'people', {
    patchBridge: (base) => ({
      ...base,
      resumeGroupCreation: async (storeId: string) => {
        resumed.push(storeId);
        return base.resumeGroupCreation(storeId);
      },
    }),
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Finish setup' }));
  });
  // Resume the existing server-side group instead of creating another.
  assert.deepEqual(resumed, ['team:homelab']);
});

test('an unavailable group displays its status and retains parent navigation', async () => {
  const rendered = await group('team:removed', 'people');
  assert.ok(rendered.getByRole('heading', { name: 'Group unavailable' }));
  // The topbar derives parent navigation from the location.
  const { parentLocation } = await import('../src/location');
  assert.deepEqual(
    parentLocation({ kind: 'group-settings', ref: 'team:removed' }),
    { kind: 'teams' },
  );
});

test('incomplete group recovery describes durable creation evidence', async () => {
  const base = await fixture();
  for (const [phase, button, detail] of [
    ['preparing', 'Continue creation', 'has not been submitted'],
    ['submission-unknown', 'Check creation', 'must be checked'],
    ['legacy-unknown', 'Check creation', 'must be checked'],
    ['remote-verified', 'Finish setup', 'was created on'],
  ] as const) {
    const snapshot = {
      ...base,
      stores: base.stores.map((store) =>
        store.id === 'team:homelab' && store.kind === 'team'
          ? { ...store, creation_phase: phase }
          : store,
      ),
    };
    const rendered = await group('team:homelab', 'people', { snapshot });
    const band = document.querySelector('.band');
    assert.ok(band);
    assert.ok(band.textContent?.includes(detail), phase);
    assert.ok(band.contains(rendered.getByRole('button', { name: button })));
    if (phase !== 'remote-verified') {
      assert.equal(band.textContent?.includes('was created on'), false);
    }
    ui.cleanup();
  }
});
