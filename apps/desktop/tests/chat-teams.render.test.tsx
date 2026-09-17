/**
 * The Chat tab's inbox column: which teams it lists, which of them are single
 * rows and which are headings with channels, what a row carries, how search
 * narrows the column, what New chat offers, and what the tab shows when no
 * team has chat.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, StrictMode, useState } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import { readSource } from './lib/source';
import type { Location } from '../src/location';
import type { AgentSnapshot } from '../src/model';
import type { Bridge } from '../src/bridge';
import type { ChatClock } from '../src/chat/inbox-service';

/** The inbox service's clock, driven by the test rather than by the wall. */
class Clock implements ChatClock {
  time = 0;
  id = 0;
  tasks = new Map<number, { due: number; fn: () => void }>();
  now = () => this.time;
  random = () => 0;
  later = (fn: () => void, delay: number) => {
    const id = ++this.id;
    this.tasks.set(id, { due: this.time + delay, fn });
    return id;
  };
  cancel = (id: unknown) => {
    this.tasks.delete(id as number);
  };
  async advance(ms: number) {
    const end = this.time + ms;
    for (;;) {
      const next = [...this.tasks].sort((a, b) => a[1].due - b[1].due)[0];
      if (!next || next[1].due > end) break;
      this.time = next[1].due;
      this.tasks.delete(next[0]);
      next[1].fn();
      for (let i = 0; i < 30; i++) await Promise.resolve();
    }
    this.time = end;
  }
}

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
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
test.after(async () => {
  await vite.close();
});

/** The fixture with chat granted on the named servers and nowhere else. */
async function snapshotWithChat(servers: string[]): Promise<AgentSnapshot> {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  return {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) => ({
      ...server,
      capabilities: {
        ...server.capabilities,
        chat: servers.includes(server.id),
      },
    })),
  };
}

/**
 * The shell's own state, for a test that has to move it while the tab is
 * mounted: the snapshot the tab reads and the per-server access generations it
 * guards chat writes on.
 */
interface Shell {
  setLocation?: (next: Location) => void;
  setSnapshot: (next: AgentSnapshot) => void;
  setGenerations: (next: ReadonlyMap<string, number>) => void;
  /** The availability clock the tab reads, which is not the service's. */
  accessNow: () => number;
}

const NO_GENERATIONS: ReadonlyMap<string, number> = new Map();

async function mount(
  snapshot: AgentSnapshot,
  start: Location,
  override?: (bridge: Bridge) => Bridge,
  clock?: ChatClock,
  shell?: Partial<Shell>,
) {
  const { ChatTab } = await vite.ssrLoadModule('/src/screens/chat-tab.tsx');
  const { ChatInboxProvider } = await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
  );
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const base = mockBridge(snapshot);
  const bridge = override?.(base) ?? base;
  const portalRoot = document.getElementById('overlays');
  if (!portalRoot) throw new Error('missing overlay root');
  const overlayRoot: HTMLElement = portalRoot;
  const journal: Location[] = [];
  function Host() {
    const [location, setLocation] = useState<Location>(start);
    // The shell owns both of these; a test that moves one moves it here.
    const [live, setLive] = useState(snapshot);
    const [generations, setGenerations] =
      useState<ReadonlyMap<string, number>>(NO_GENERATIONS);
    if (shell) {
      shell.setLocation = setLocation;
      shell.setSnapshot = setLive;
      shell.setGenerations = setGenerations;
    }
    return createElement(
      StrictMode,
      null,
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot: overlayRoot,
        children: createElement(ChatInboxProvider, {
          bridge,
          snapshot: live,
          clock,
          children: createElement(ChatTab, {
            snapshot: live,
            bridge,
            location,
            accessNow: shell?.accessNow,
            accessGenerations: generations,
            onNavigate: (next: Location) => {
              journal.push(next);
              if (next.kind === 'chat') setLocation(next);
            },
          }),
        }),
      }),
    );
  }
  ui.render(createElement(Host));
  return journal;
}

/** Every row that names a team, whether a conversation row or a heading. */
function heads(): HTMLElement[] {
  return [
    ...document.querySelectorAll<HTMLElement>('.chat-conv, .chat-team-head'),
  ];
}

function head(name: string): HTMLElement {
  const row = heads().find(
    (node) => node.querySelector('b')?.textContent === name,
  );
  assert.ok(row, `${name} is in the column`);
  return row;
}

/** Channel rows, by the `#name` each draws. */
function channels(): string[] {
  return [...document.querySelectorAll('.chat-channel .n')].map(
    (node) => node.textContent ?? '',
  );
}

/**
 * A bridge that gives one team more channels than the general channel the mock
 * agent starts with, so the column draws it as a heading.
 */
function withChannels(
  base: Bridge,
  store: string,
  extra: {
    id: string;
    name: string;
    admin?: boolean;
    unread?: string;
    muted?: boolean;
    hidden?: boolean;
  }[],
): Bridge {
  return {
    ...base,
    chat: async (id, action, view) => {
      const reply = await base.chat(id, action, view);
      if (id !== store || reply.result.kind !== 'inbox') return reply;
      const template = reply.result.channels[0];
      for (const row of extra) {
        const channel = {
          ...template,
          id: row.id,
          name: row.name,
          admin: row.admin ?? false,
        };
        reply.result.channels = [...reply.result.channels, channel];
        reply.result.conversations = [
          ...reply.result.conversations,
          {
            channel,
            inbox_version: '1',
            read_through: '0',
            pending_read: null,
            unread: row.unread ?? '0',
            hidden: row.hidden ?? false,
            muted: row.muted ?? false,
            preview: null,
          },
        ];
      }
      return reply;
    },
  };
}

/** One team's inbox reply, with the flags the agent sets on it rewritten. */
function withInbox(
  base: Bridge,
  store: string,
  patch: (
    inbox: Extract<
      Awaited<ReturnType<Bridge['chat']>>['result'],
      { kind: 'inbox' }
    >,
  ) => void,
): Bridge {
  return {
    ...base,
    chat: async (id, action, view) => {
      const reply = await base.chat(id, action, view);
      if (id === store && reply.result.kind === 'inbox') patch(reply.result);
      return reply;
    },
  };
}

/** The channel row drawing `#name`, wherever in the column it sits. */
function channelRow(name: string): HTMLButtonElement {
  const row = [
    ...document.querySelectorAll<HTMLButtonElement>('.chat-channel'),
  ].find((node) => node.querySelector('.n')?.textContent === name);
  assert.ok(row, `${name} is in the column`);
  return row;
}

test('the tab with no conversation opens the most recent one', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  // Household's only message is newer than Engineering's, so Household opens
  // even though Engineering comes first in the navigation order.
  const journal = await mount(snapshot, { kind: 'chat' }, (base) => ({
    ...base,
    chat: async (store, action, view) => {
      const reply = await base.chat(store, action, view);
      if (store === 'team:household' && reply.result.kind === 'inbox')
        reply.result.conversations = reply.result.conversations.map((c) => ({
          ...c,
          preview: c.preview
            ? { ...c.preview, insert_time: '1700000009999' }
            : null,
        }));
      return reply;
    },
  }));
  await ui.waitFor(() =>
    assert.deepEqual(journal.at(0), {
      kind: 'chat',
      ref: 'team:household',
      channel: 'ab'.repeat(16),
    }),
  );
  await ui.screen.findByText(
    'Chat opened Household · #general because no conversation was selected.',
  );
  // The row a conversation is open in is the current one; the rest are not.
  await ui.waitFor(() =>
    assert.equal(head('Household').getAttribute('aria-current'), 'page'),
  );
  assert.equal(head('Engineering').getAttribute('aria-current'), null);
});

test('with no message anywhere the tab opens the first team that has chat', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const journal = await mount(snapshot, { kind: 'chat' }, (base) => ({
    ...base,
    chat: async (store, action, view) => {
      const reply = await base.chat(store, action, view);
      if (reply.result.kind === 'inbox')
        reply.result.conversations = reply.result.conversations.map((c) => ({
          ...c,
          preview: null,
        }));
      return reply;
    },
  }));
  await ui.waitFor(() =>
    assert.deepEqual(journal.at(0), { kind: 'chat', ref: 'team:eng' }),
  );
  await ui.screen.findByText(
    'Chat opened Engineering because no team was selected.',
  );
});

test('every team with chat is listed at once, single channel sets as one row', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' }, (base) =>
    withChannels(base, 'team:household', [
      { id: '11'.repeat(16), name: 'chores', unread: '3' },
      { id: '22'.repeat(16), name: 'incidents', admin: true },
    ]),
  );
  // Household has three channels, so it is a heading with its channels under
  // it; Engineering has only the general channel, so it is one row.
  await ui.waitFor(() =>
    assert.deepEqual(channels(), ['#general', '#chores', '#incidents']),
  );
  assert.ok(head('Household').classList.contains('chat-team-head'));
  assert.ok(head('Engineering').classList.contains('chat-conv'));
  const household = head('Household').closest('.chat-team');
  assert.ok(
    [...document.querySelectorAll('.chat-channel')].every((row) =>
      household?.contains(row),
    ),
    'channels sit under the team that owns them',
  );
  // The per-channel count is the channel's own, and an admin channel says so.
  const chores = [...document.querySelectorAll('.chat-channel')].find((row) =>
    row.textContent?.startsWith('#chores'),
  );
  assert.equal(
    chores?.querySelector('.chat-unread')?.getAttribute('aria-label'),
    '3 unread',
  );
  const incidents = [...document.querySelectorAll('.chat-channel')].find(
    (row) => row.textContent?.startsWith('#incidents'),
  );
  assert.equal(
    incidents?.querySelector('.lock')?.getAttribute('aria-label'),
    'Admins and owners only',
  );
});

test('search narrows the whole column to matching teams and channels', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' }, (base) =>
    withChannels(base, 'team:household', [
      { id: '11'.repeat(16), name: 'chores' },
    ]),
  );
  await ui.waitFor(() => assert.deepEqual(channels(), ['#general', '#chores']));
  const field = ui.screen.getByRole('searchbox', {
    name: 'Search teams and channels',
  });
  ui.fireEvent.change(field, { target: { value: 'chores' } });
  // A channel match keeps its team's heading and drops the rest of the column.
  await ui.waitFor(() => assert.deepEqual(channels(), ['#chores']));
  assert.deepEqual(
    heads().map((row) => row.querySelector('b')?.textContent),
    ['Household'],
  );
  // A team match keeps all of that team's channels.
  ui.fireEvent.change(field, { target: { value: 'engine' } });
  await ui.waitFor(() =>
    assert.deepEqual(
      heads().map((row) => row.querySelector('b')?.textContent),
      ['Engineering'],
    ),
  );
  ui.fireEvent.change(field, { target: { value: 'nothing here' } });
  await ui.screen.findByText('No team or channel matches “nothing here”.');
});

test('switching teams keeps the column and mounts exactly one conversation', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  // Everything the mounted conversation asks of a team, as opposed to the
  // inbox polling the column reads for every team.
  const conversation: string[] = [];
  const clock = new Clock();
  const journal = await mount(
    snapshot,
    { kind: 'chat', ref: 'team:household', channel: 'ab'.repeat(16) },
    (base) => ({
      ...base,
      chat: async (store, action, view) => {
        if (['history', 'pending', 'mark-read'].includes(action.action))
          conversation.push(`${store}:${action.action}`);
        return base.chat(store, action, view);
      },
    }),
    clock,
  );
  await clock.advance(1000);
  await ui.waitFor(() => assert.ok(heads().length));
  assert.equal(head('Engineering').getAttribute('aria-current'), null);
  const column = document.querySelector('.chat-inbox');
  await ui.waitFor(() =>
    assert.ok(conversation.some((call) => call.startsWith('team:household'))),
  );
  ui.fireEvent.click(head('Engineering'));
  assert.deepEqual(journal.at(-1), {
    kind: 'chat',
    ref: 'team:eng',
    channel: 'ab'.repeat(16),
  });
  await ui.waitFor(() =>
    assert.equal(head('Engineering').getAttribute('aria-current'), 'page'),
  );
  assert.equal(head('Household').getAttribute('aria-current'), null);
  // The column is the tab's, not the conversation's: the switch replaced the
  // conversation beside it without rebuilding it.
  assert.equal(document.querySelector('.chat-inbox'), column);
  // Exactly one conversation is mounted: once Engineering's has settled, the
  // team that was open is asked for nothing more.
  await ui.waitFor(() =>
    assert.ok(conversation.some((call) => call.startsWith('team:eng'))),
  );
  const settled = conversation.length;
  // A minute of the service's clock, the only clock a mounted conversation
  // follows, passes without Household being asked for anything.
  await clock.advance(60_000);
  assert.deepEqual(
    conversation
      .slice(settled)
      .filter((call) => call.startsWith('team:household')),
    [],
  );
});

test('a team with no conversation mounted carries its preview and unread count', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' }, (base) => ({
    ...base,
    chat: async (store, action, view) => {
      const reply = await base.chat(store, action, view);
      if (reply.result.kind === 'inbox')
        reply.result.conversations = reply.result.conversations.map(
          (conversation) => ({ ...conversation, unread: '2' }),
        );
      return reply;
    },
  }));
  // No conversation is mounted for Household: its preview, its time and its
  // count all come from the inbox service the rail already reads. The badge
  // sits inside the row that names the team, so its label is the count alone.
  const badge = await ui.waitFor(() => {
    const node = head('Household').querySelector('.chat-unread');
    assert.ok(node, 'Household carries an unread badge');
    return node;
  });
  assert.equal(badge.getAttribute('aria-label'), '2 unread');
  assert.equal(badge.textContent, '2');
  assert.equal(
    head('Household').querySelector('small:not(.chat-row-identity)')
      ?.textContent,
    'Team member: Team chat is ready.',
  );
  assert.ok(head('Household').querySelector('.when')?.textContent);
});

test('a team whose server offers no chat sits under No chat with the reason', async () => {
  const snapshot = await snapshotWithChat(['personal']);
  await mount(snapshot, { kind: 'chat' });
  await ui.waitFor(() => assert.ok(heads().length));
  const engineering = head('Engineering');
  assert.ok(engineering.classList.contains('off'));
  assert.equal(engineering.getAttribute('role'), null);
  // One group, one mark: the column draws the group's own mark, the same one
  // the Teams row and the group's page draw, not a generic people glyph.
  const mark = engineering.querySelector('.kico.group');
  assert.ok(mark);
  assert.equal(mark.textContent, 'E');
  // The initial stands for the name beside it, so it is not read out twice.
  assert.equal(mark.getAttribute('aria-hidden'), 'true');
  assert.match(
    engineering.querySelector('small:not(.chat-row-identity)')?.textContent ??
      '',
    /^Chat is not enabled on /,
  );
  const labels = [...document.querySelectorAll('.sec')].map(
    (node) => node.textContent,
  );
  assert.ok(labels.includes('Conversations'));
  assert.ok(labels.includes('Chat unavailable'));
});

test('a team with a lapsed server check-in remains listed with recovery actions', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/lease.ts',
  )) as typeof import('../src/model/lease');
  // Engineering's server offers chat but its check-in has expired, so the team
  // is listed and locked rather than filed under "No chat".
  const snapshot = applyLease(
    await snapshotWithChat(['personal', 'acme']),
    'lapsed',
    'acme',
  );
  const journal = await mount(snapshot, { kind: 'chat', ref: 'team:eng' });
  await ui.waitFor(() => assert.ok(heads().length));
  const engineering = head('Engineering');
  assert.equal(engineering.getAttribute('aria-current'), 'page');
  // The row states the reason rather than a preview it cannot have, in the
  // words the rest of the shell uses for that store.
  assert.equal(
    engineering.querySelector('small:not(.chat-row-identity)')?.textContent,
    'Check-in expired',
  );
  assert.equal(
    engineering.querySelector('.chat-unread')?.getAttribute('aria-label'),
    'Check-in expired',
  );
  assert.equal(engineering.querySelector('.chat-unread')?.textContent, '!');
  // A locked team is opened onto the pane that says why it is locked; the
  // channel rows a team keeps while it is out of reach are the next test.
  await ui.screen.findByRole('heading', { name: 'Engineering chat is locked' });
  ui.fireEvent.click(head('Household'));
  await ui.waitFor(() =>
    assert.equal(head('Household').getAttribute('aria-current'), 'page'),
  );
  ui.fireEvent.click(head('Engineering'));
  await ui.screen.findByRole('heading', { name: 'Engineering chat is locked' });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Check in' }));
  assert.deepEqual(journal.at(-1), {
    kind: 'settings',
    section: 'servers',
    profile: 'acme',
  });
  // Open another available team.
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Open another team' }),
  );
  assert.deepEqual(journal.at(-1), { kind: 'chat', ref: 'team:household' });
});

test('a channel row of a team that went out of reach opens the locked pane', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  // The tab reads the availability clock on every render; the inbox service
  // reads it when the store list changes. A check-in that lapses in between is
  // a team whose channels are listed and whose chat is out of reach — the one
  // state in which a locked team has channel rows at all.
  const lapsed = Math.floor(Date.now() / 1000) + 13 * 86_400;
  const journal = await mount(
    snapshot,
    { kind: 'chat', ref: 'team:household' },
    (base) =>
      withChannels(base, 'team:eng', [
        { id: '33'.repeat(16), name: 'deploys' },
      ]),
    undefined,
    { accessNow: () => lapsed },
  );
  const deploys = await ui.waitFor(() => channelRow('#deploys'));
  // Neither the row nor the single-channel team's row is inert: both open the
  // pane that states the reason, which is more than an inert row can say.
  assert.equal(deploys.disabled, false);
  assert.ok(deploys.classList.contains('off'));
  ui.fireEvent.click(deploys);
  assert.deepEqual(journal.at(-1), {
    kind: 'chat',
    ref: 'team:eng',
    channel: '33'.repeat(16),
  });
  await ui.screen.findByRole('heading', { name: 'Engineering chat is locked' });
});

test('with no team at all the tab offers team creation', async () => {
  const snapshot = await snapshotWithChat([]);
  const journal = await mount(snapshot, { kind: 'chat' });
  await ui.screen.findByRole('heading', { name: 'No team chats yet' });
  assert.equal(ui.screen.queryByText('How chat gets turned on'), null);
  // The "Chat unavailable" teams are still a column worth searching, so the field is
  // live even though New chat has no team to offer.
  const field = ui.screen.getByRole<HTMLInputElement>('searchbox', {
    name: 'Search teams and channels',
  });
  assert.equal(field.disabled, false);
  ui.fireEvent.change(field, { target: { value: 'engine' } });
  await ui.waitFor(() =>
    assert.deepEqual(
      heads().map((row) => row.querySelector('b')?.textContent),
      ['Engineering'],
    ),
  );
  ui.fireEvent.change(field, { target: { value: 'nothing here' } });
  await ui.screen.findByText('No team or channel matches “nothing here”.');
  ui.fireEvent.change(field, { target: { value: '' } });
  // Nothing is selected, so nothing is navigated to.
  assert.equal(journal.length, 0);
});

test('the note names the conversation the tab chose and leaves focus in the pane', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(snapshot, { kind: 'chat' });
  // Both teams' only messages arrived at the same moment, so the tie keeps
  // navigation order and Engineering is the conversation that opens.
  await ui.screen.findByText(
    'Chat opened Engineering · #general because no conversation was selected.',
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Dismiss' }));
  await ui.waitFor(() =>
    assert.equal(ui.screen.queryByText(/no conversation was selected/), null),
  );
  // The dismissed note took focus with it, so the conversation takes it back.
  assert.equal(
    document.activeElement?.getAttribute('aria-label'),
    'Conversation',
  );
});

test('picking a team ends the note, and returning does not bring it back', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(snapshot, { kind: 'chat' });
  await ui.screen.findByText(/no conversation was selected/);
  // An explicit selection removes the automatic-selection note permanently.
  ui.fireEvent.click(head('Household'));
  await ui.waitFor(() =>
    assert.equal(head('Household').getAttribute('aria-current'), 'page'),
  );
  assert.equal(ui.screen.queryByText(/no conversation was selected/), null);
  ui.fireEvent.click(head('Engineering'));
  await ui.waitFor(() =>
    assert.equal(head('Engineering').getAttribute('aria-current'), 'page'),
  );
  assert.equal(ui.screen.queryByText(/no conversation was selected/), null);
});

test('unfinished work sits in a bounded section and keeps the composer', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  let bridge: Bridge | undefined;
  await mount(
    snapshot,
    { kind: 'chat', ref: 'team:eng', channel: 'ab'.repeat(16) },
    (base) => {
      bridge = base;
      return base;
    },
  );
  await ui.screen.findByText('Team chat is ready.');
  assert.ok(bridge);
  // Six preparations no one finished: enough that an unbounded section would
  // push the thread and the composer out of the pane.
  for (let index = 0; index < 6; index++)
    await bridge.chat(
      'team:eng',
      {
        action: 'prepare-channel',
        submission: `pending-${index}`,
        name: `channel${index}`,
        description: '',
        admin: false,
      },
      'test',
    );
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh messages' }),
  );
  await ui.waitFor(() =>
    assert.equal(document.querySelectorAll('.chat-pending').length, 6),
  );
  const conversation = document.querySelector('.chat-conversation');
  const recovery = document.querySelector('.chat-recovery');
  const composer = document.querySelector('.chat-composer');
  assert.ok(recovery && composer, 'the composer stays beside the saved work');
  assert.equal(recovery.parentElement, conversation);
  assert.ok(conversation?.contains(composer));
  // The section is bounded and scrolls, so the rows cannot take the pane.
  const css = await readSource('../src/screens/chat.css', import.meta.url);
  const rule = /\.chat-recovery \{([^}]*)\}/.exec(css)?.[1] ?? '';
  assert.match(rule, /overflow:\s*auto/);
  assert.match(rule, /max-height:\s*\d+%/);
  assert.doesNotMatch(rule, /flex:\s*none/);
});

test('saved work that is accounted for says so rather than vanishing', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  let bridge: Bridge | undefined;
  await mount(
    snapshot,
    { kind: 'chat', ref: 'team:eng', channel: 'ab'.repeat(16) },
    (base) => {
      bridge = base;
      return base;
    },
  );
  await ui.screen.findByText('Team chat is ready.');
  assert.ok(bridge);
  await bridge.chat(
    'team:eng',
    {
      action: 'prepare-channel',
      submission: 'pending-caught-up',
      name: 'abandoned',
      description: '',
      admin: false,
    },
    'test',
  );
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh messages' }),
  );
  await ui.screen.findByText('Needs attention');
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Cancel preparation' }),
  );
  await ui.screen.findByText(
    'All caught up. Everything saved on this device has been accounted for.',
  );
  assert.equal(ui.screen.queryByText('Needs attention'), null);
});

test('the rail returns to the chat the tab last had open', async () => {
  const { sidebarCycleLocations } = (await vite.ssrLoadModule(
    '/src/shell/sidebar.tsx',
  )) as typeof import('../src/shell/sidebar');
  const { railTabOf, rememberChatLocation } = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  /** Where the rail's Chat tab goes, whatever its place in the cycle. */
  const railChat = () =>
    sidebarCycleLocations().find((place) => railTabOf(place) === 'chat');
  const channel = 'ab'.repeat(16);
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  rememberChatLocation(null);
  await mount(snapshot, { kind: 'chat', ref: 'team:household', channel });
  await ui.waitFor(() =>
    assert.deepEqual(railChat(), {
      kind: 'chat',
      ref: 'team:household',
      channel,
    }),
  );
  // A location naming a team with no chat says so in the pane; a memory of a
  // team that still has chat is not collateral damage.
  ui.cleanup();
  rememberChatLocation({ kind: 'chat', ref: 'team:household', channel });
  await mount(await snapshotWithChat(['personal']), {
    kind: 'chat',
    ref: 'team:eng',
  });
  await ui.screen.findByRole('heading', { name: 'Engineering has no chat' });
  assert.deepEqual(railChat(), {
    kind: 'chat',
    ref: 'team:household',
    channel,
  });
  // The remembered team losing chat is what forgets it.
  ui.cleanup();
  rememberChatLocation({ kind: 'chat', ref: 'team:household', channel });
  await mount(await snapshotWithChat([]), { kind: 'chat', ref: 'team:eng' });
  await ui.waitFor(() => assert.deepEqual(railChat(), { kind: 'chat' }));
});

test('New chat picks a team, states why one cannot be picked, and opens a channel', async () => {
  const snapshot = await snapshotWithChat(['personal']);
  const journal = await mount(snapshot, { kind: 'chat' });
  await ui.waitFor(() => assert.ok(heads().length));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'New chat' }));
  const sheet = await ui.screen.findByRole('dialog', { name: 'New chat' });
  assert.ok(sheet);
  // A team whose server offers no chat is offered inert with its reason, not
  // hidden: the reason is what the reader needs, so the card is marked
  // `aria-disabled` rather than `disabled` and the keyboard still reaches it.
  const engineering = ui.screen.getByRole('radio', { name: /^Engineering/ });
  assert.equal((engineering as HTMLButtonElement).disabled, false);
  assert.equal(engineering.getAttribute('aria-disabled'), 'true');
  assert.equal(engineering.tabIndex, 0);
  assert.match(engineering.textContent ?? '', /Chat is not enabled on /);
  // Inert means inert: pressing it does not choose the team.
  ui.fireEvent.click(engineering);
  assert.equal(engineering.getAttribute('aria-checked'), 'false');
  ui.fireEvent.click(ui.screen.getByRole('radio', { name: /^Household/ }));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Continue' }));
  ui.fireEvent.click(await ui.screen.findByRole('radio', { name: /#general/ }));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Open chat' }));
  await ui.waitFor(() => assert.equal(ui.screen.queryByRole('dialog'), null));
  assert.deepEqual(journal.at(-1), {
    kind: 'chat',
    ref: 'team:household',
    channel: 'ab'.repeat(16),
  });
});

test('New chat creates a channel, refusing a name the agent would refuse', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const journal = await mount(snapshot, { kind: 'chat', ref: 'team:eng' });
  await ui.waitFor(() => assert.ok(heads().length));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'New chat' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('radio', { name: /^Engineering/ }),
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Continue' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('radio', { name: /Create a channel/ }),
  );
  const name = ui.screen.getByRole('textbox', { name: 'Channel name' });
  const create = () =>
    ui.screen.getByRole('button', { name: /Create channel/ });
  ui.fireEvent.change(name, { target: { value: 'ab' } });
  await ui.screen.findByText('Channel names are at least 3 characters.');
  assert.equal((create() as HTMLButtonElement).disabled, true);
  // The general channel has no name of its own, so "general" is refused with
  // the instruction that works.
  ui.fireEvent.change(name, { target: { value: 'general' } });
  await ui.screen.findByText(
    'Leave the name empty to create the general channel.',
  );
  ui.fireEvent.change(name, { target: { value: 'design' } });
  await ui.screen.findByText('Created as #design.');
  ui.fireEvent.click(create());
  await ui.screen.findByRole('button', { name: /#design/ });
  await ui.waitFor(() => assert.equal(ui.screen.queryByRole('dialog'), null));
  assert.deepEqual(journal.at(-1), {
    kind: 'chat',
    ref: 'team:eng',
    channel: '0000000000000000000000000000000b',
  });
});

test('a synchronization that succeeded but could not finish keeps its preview', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  // Both flags are soft: the inbox arrived, so the column still has previews,
  // channels and counts. Engineering is the open team, whose row is the one
  // that used to be reduced to "Channels unavailable".
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' }, (base) =>
    withInbox(
      withInbox(base, 'team:eng', (inbox) => {
        inbox.read_retry_pending = true;
      }),
      'team:household',
      (inbox) => {
        inbox.previews_incomplete = true;
      },
    ),
  );
  await ui.waitFor(() =>
    assert.equal(
      head('Engineering').querySelector('small:not(.chat-row-identity)')
        ?.textContent,
      'Team member: Team chat is ready.',
    ),
  );
  const engineering = head('Engineering');
  // The preview stays where it is; what could not be finished is a caption
  // beside it, not a failure in place of it.
  assert.equal(
    engineering.querySelector('.chat-row-note')?.textContent,
    'Read status will retry.',
  );
  assert.equal(
    [...engineering.querySelectorAll('small')].some(
      (node) => node.textContent === 'Channels unavailable',
    ),
    false,
  );
  assert.ok(engineering.querySelector('.when')?.textContent);
  // A team that is not the open one keeps its preview just the same.
  const household = head('Household');
  assert.equal(
    household.querySelector('small:not(.chat-row-identity)')?.textContent,
    'Team member: Team chat is ready.',
  );
  assert.equal(
    household.querySelector('.chat-row-note')?.textContent,
    'Some previews are unavailable.',
  );
});

test('a heading team whose count is degraded carries the badge that says so', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' }, (base) =>
    withInbox(
      withInbox(
        withChannels(base, 'team:household', [
          { id: '11'.repeat(16), name: 'chores' },
        ]),
        'team:household',
        (inbox) => {
          inbox.degraded = true;
        },
      ),
      'team:eng',
      (inbox) => {
        // Engineering's one channel has a name of its own, so it is a heading
        // too: a row that stood in for it would lose "#deploys".
        inbox.channels = inbox.channels.map((channel) => ({
          ...channel,
          name: 'deploys',
        }));
      },
    ),
  );
  await ui.waitFor(() =>
    assert.deepEqual(channels(), ['#deploys', '#general', '#chores']),
  );
  assert.ok(head('Engineering').classList.contains('chat-team-head'));
  // Household is a heading, and its badge is not a count the channels beneath
  // it already carry — it is the state only the team can report.
  const heading = head('Household');
  assert.ok(heading.classList.contains('chat-team-head'));
  const badge = heading.querySelector('.chat-unread');
  assert.ok(badge, 'the heading says its count is incomplete');
  assert.equal(badge.textContent, '1+');
  assert.equal(
    badge.getAttribute('aria-label'),
    '1 known unread; inbox synchronization incomplete',
  );
  // The team name is a heading over its channels, and the channels say which
  // team they belong to.
  const name = heading.querySelector('b');
  assert.equal(name?.getAttribute('role'), 'heading');
  assert.equal(name?.getAttribute('aria-level'), '3');
  const group = ui.screen.getByRole('group', { name: 'Household' });
  assert.ok(group.contains(channelRow('#chores')));
});

test('muted and hidden conversations stay listed and say what they are', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(snapshot, { kind: 'chat', ref: 'team:household' }, (base) =>
    withInbox(
      withChannels(base, 'team:household', [
        { id: '11'.repeat(16), name: 'chores', unread: '3', muted: true },
        { id: '22'.repeat(16), name: 'archive', hidden: true },
      ]),
      'team:eng',
      (inbox) => {
        // Engineering's only channel is the general one, so the team is a
        // single row: the row is the channel, muted count and all.
        inbox.conversations = inbox.conversations.map((conversation) => ({
          ...conversation,
          muted: true,
          unread: '2',
        }));
      },
    ),
  );
  // A hidden conversation keeps its channel listed rather than taking it away.
  await ui.waitFor(() =>
    assert.deepEqual(channels(), ['#general', '#chores', '#archive']),
  );
  const chores = channelRow('#chores');
  assert.equal(
    chores.querySelector('small:not(.chat-row-identity)')?.textContent,
    'Muted',
  );
  const choresBadge = chores.querySelector('.chat-unread');
  assert.equal(choresBadge?.textContent, '3');
  assert.ok(choresBadge?.classList.contains('muted'));
  // The row is dimmed the way a hidden one is, so the caption and the look
  // agree rather than only the badge being quiet.
  assert.ok(chores.classList.contains('muted'));
  assert.ok(channelRow('#archive').classList.contains('hidden'));
  assert.equal(
    channelRow('#archive').querySelector('small:not(.chat-row-identity)')
      ?.textContent,
    'Hidden',
  );
  // The single-row team draws the same things its channel row would: the count
  // it is bold for, and the caption that says why the count is quiet.
  const engineering = head('Engineering');
  assert.ok(engineering.classList.contains('chat-conv'));
  assert.ok(engineering.classList.contains('unread'));
  assert.equal(
    engineering.querySelector('.chat-row-note')?.textContent,
    'Muted',
  );
  const badge = engineering.querySelector('.chat-unread');
  assert.equal(badge?.textContent, '2');
  assert.equal(badge?.getAttribute('aria-label'), '2 unread');
  assert.ok(badge?.classList.contains('muted'));
  assert.ok(engineering.classList.contains('muted'));
});

test('a conversation opened into a heading team marks the channel row it mounts', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const incidents = '22'.repeat(16);
  const withRooms = (base: Bridge) =>
    withChannels(base, 'team:household', [
      { id: '11'.repeat(16), name: 'chores' },
      { id: incidents, name: 'incidents' },
    ]);
  // A notification activation arrives as a location naming the team and the
  // channel: the row it opens is the current one, not the team's first.
  await mount(
    snapshot,
    { kind: 'chat', ref: 'team:household', channel: incidents },
    withRooms,
  );
  await ui.waitFor(() =>
    assert.equal(channelRow('#incidents').getAttribute('aria-current'), 'page'),
  );
  assert.equal(channelRow('#general').getAttribute('aria-current'), null);
  // The pane mounted the same channel the row is marked for.
  assert.equal(
    document.querySelector('.chat-thread-title h2')?.textContent,
    'Household · #incidents',
  );
  // The tab's own choice marks its row on the render that makes it, rather
  // than marking the team's first channel until the location catches up.
  ui.cleanup();
  const journal = await mount(snapshot, { kind: 'chat' }, (base) =>
    withInbox(withRooms(base), 'team:household', (inbox) => {
      inbox.conversations = inbox.conversations.map((conversation) =>
        conversation.channel.id === incidents
          ? {
              ...conversation,
              preview: {
                sender: '',
                sequence: '1',
                send_time: '1700000009999',
                insert_time: '1700000009999',
                content: { kind: 'text' as const, text: 'The build is red.' },
              },
            }
          : { ...conversation, preview: null },
      );
    }),
  );
  await ui.waitFor(() =>
    assert.deepEqual(journal.at(-1), {
      kind: 'chat',
      ref: 'team:household',
      channel: incidents,
    }),
  );
  assert.equal(channelRow('#incidents').getAttribute('aria-current'), 'page');
});

test('the pane waits while every reachable team is on its first synchronization', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  let release = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const journal = await mount(snapshot, { kind: 'chat' }, (base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'sync-inbox') await gate;
      return base.chat(store, action, view);
    },
  }));
  // Nothing has answered, so the answer is not knowable yet: the tab says so
  // rather than opening a team it would have to leave.
  await ui.screen.findByText('Loading conversations…');
  assert.equal(journal.length, 0);
  assert.equal(ui.screen.queryByRole('heading', { name: /has no chat/ }), null);
  release();
  // The wait ends the moment a team answers.
  await ui.waitFor(() => assert.ok(journal.length));
  assert.equal(journal.at(-1)?.kind, 'chat');
  await ui.waitFor(() =>
    assert.equal(ui.screen.queryByText('Loading conversations…'), null),
  );
});

test('an interrupted attempt recovers the same preparation rather than a second', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const submissions: string[] = [];
  const attempts: string[] = [];
  const journal = await mount(
    snapshot,
    { kind: 'chat', ref: 'team:eng', channel: 'ab'.repeat(16) },
    (base) => ({
      ...base,
      chat: async (store, action, view) => {
        if (action.action === 'prepare-channel')
          submissions.push(action.submission);
        if (action.action === 'attempt' && store === 'team:eng') {
          attempts.push(action.operation);
          if (attempts.length === 1)
            throw {
              code: 'transport',
              message: 'The reply was lost.',
              retryable: true,
              fatal: false,
              ambiguous: true,
            };
        }
        return base.chat(store, action, view);
      },
    }),
  );
  await ui.screen.findByText('Team chat is ready.');
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'New chat' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('radio', { name: /^Engineering/ }),
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Continue' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('radio', { name: /Create a channel/ }),
  );
  ui.fireEvent.change(
    ui.screen.getByRole('textbox', { name: 'Channel name' }),
    { target: { value: 'design' } },
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: /Create channel/ }));
  // The agent has taken the preparation, so what is offered is a recovery of
  // that one rather than another run at creating the channel.
  const recover = await ui.screen.findByRole('button', {
    name: 'Retry channel creation',
  });
  await ui.screen.findByText('The reply was lost.');
  await ui.screen.findByText(
    /Select Retry to resend the request without creating a duplicate/,
  );
  ui.fireEvent.click(recover);
  await ui.screen.findByRole('button', { name: /#design/ });
  await ui.waitFor(() => assert.equal(ui.screen.queryByRole('dialog'), null));
  // One submission, one operation, attempted twice: the channel the reader
  // gets is the one that was prepared, not a second one beside it.
  assert.equal(submissions.length, 1);
  assert.equal(attempts.length, 2);
  assert.equal(attempts[0], attempts[1]);
  assert.deepEqual(journal.at(-1), {
    kind: 'chat',
    ref: 'team:eng',
    channel: attempts[0],
  });
});

/** New chat → a team → Create a channel, the way the column offers it. */
async function openCreateForm(team: RegExp): Promise<HTMLElement> {
  ui.fireEvent.click(ui.screen.getAllByRole('button', { name: 'New chat' })[0]);
  ui.fireEvent.click(await ui.screen.findByRole('radio', { name: team }));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Continue' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('radio', { name: /Create a channel/ }),
  );
  return ui.screen.getByRole('textbox', { name: 'Channel name' });
}

/** A bridge that holds the first request of one kind until it is released. */
function holdingFirst(
  base: Bridge,
  kind: string,
  seen: (
    action: Extract<Parameters<Bridge['chat']>[1], { action: string }>,
  ) => void = () => {},
): { bridge: Bridge; reached: Promise<void>; release: () => void } {
  let arrived!: () => void;
  const reached = new Promise<void>((resolve) => {
    arrived = resolve;
  });
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let first = true;
  return {
    reached,
    release: () => release(),
    bridge: {
      ...base,
      chat: async (store, action, view) => {
        if (action.action === kind) {
          seen(action);
          if (first) {
            first = false;
            arrived();
            await gate;
          }
        }
        return base.chat(store, action, view);
      },
    },
  };
}

test('newer messages in other teams do not switch away from the active conversation', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const clock = new Clock();
  let newer = false;
  const journal = await mount(
    snapshot,
    { kind: 'chat' },
    (base) => ({
      ...base,
      chat: async (store, action, view) => {
        const reply = await base.chat(store, action, view);
        if (
          newer &&
          store === 'team:household' &&
          reply.result.kind === 'inbox'
        )
          reply.result.conversations = reply.result.conversations.map((c) => ({
            ...c,
            preview: c.preview
              ? { ...c.preview, insert_time: '1900000000000' }
              : null,
          }));
        return reply;
      },
    }),
    clock,
  );
  await clock.advance(1000);
  // The tie kept navigation order, so the tab opened Engineering and said so.
  await ui.screen.findByText(/Chat opened Engineering/);
  const chosen = journal.length;
  // A message arrives in the other team. The tab's choice was provisional only
  // while the inbox it was made from was still filling in; it has landed, and
  // the note standing is not a licence to move the reader.
  newer = true;
  await clock.advance(60_000);
  assert.equal(journal.length, chosen);
  assert.deepEqual(journal.at(-1), {
    kind: 'chat',
    ref: 'team:eng',
    channel: 'ab'.repeat(16),
  });
  assert.equal(head('Engineering').getAttribute('aria-current'), 'page');
});

test('New chat waits for a team’s channels before either step can be answered', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const held = { current: null as null | (() => void) };
  let release = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  held.current = release;
  await mount(snapshot, { kind: 'chat', ref: 'team:household' }, (base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'sync-inbox' && store === 'team:eng') await gate;
      return base.chat(store, action, view);
    },
  }));
  await ui.waitFor(() => assert.ok(heads().length));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'New chat' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('radio', { name: /^Engineering/ }),
  );
  // An empty name is the general channel, and whether the team already has one
  // is the difference between creating it and being refused: until the channel
  // list arrives the step cannot be answered.
  const step = () =>
    ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Continue' });
  assert.equal(step().disabled, true);
  release();
  await ui.waitFor(() => assert.equal(step().disabled, false));
  ui.fireEvent.click(step());
  await ui.screen.findByRole('radio', { name: /Create a channel/ });
});

test('a team that goes out of reach while step two is open refuses it and says why', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/lease.ts',
  )) as typeof import('../src/model/lease');
  const fresh = await snapshotWithChat(['personal', 'acme']);
  const shell: Partial<Shell> = {};
  await mount(
    fresh,
    { kind: 'chat', ref: 'team:household' },
    undefined,
    undefined,
    shell,
  );
  await ui.waitFor(() => assert.ok(heads().length));
  const name = await openCreateForm(/^Engineering/);
  ui.fireEvent.change(name, { target: { value: 'design' } });
  await ui.act(async () => {
    shell.setSnapshot?.(applyLease(fresh, 'lapsed', 'acme'));
  });
  // The reason stands where the channels would be, and nothing is submitted
  // against a team this Mac cannot reach.
  await ui.screen.findByText(
    'Chat is currently unavailable for Engineering: Check-in expired',
  );
  await ui.screen.findByText('Loading channels…');
  assert.equal(
    ui.screen.getByRole<HTMLButtonElement>('button', {
      name: /Create channel/,
    }).disabled,
    true,
  );
});

test('each step of New chat takes focus, and Back returns it to the step', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' });
  await ui.waitFor(() => assert.ok(heads().length));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'New chat' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('radio', { name: /^Engineering/ }),
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Continue' }));
  // A step replaces the whole body, so focus follows it rather than staying on
  // a control that is no longer there.
  const general = await ui.screen.findByRole('radio', { name: /#general/ });
  await ui.waitFor(() => assert.equal(document.activeElement, general));
  ui.fireEvent.click(
    ui.screen.getByRole('radio', { name: /Create a channel/ }),
  );
  const name = ui.screen.getByRole('textbox', { name: 'Channel name' });
  await ui.waitFor(() => assert.equal(document.activeElement, name));
  // Back out of the create form is a step change too: focus lands in the step
  // it returns to rather than on the document.
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Back' }));
  await ui.waitFor(() =>
    assert.equal(
      document.activeElement,
      ui.screen.getByRole('radio', { name: /#general/ }),
    ),
  );
});

test('New chat refuses what the agent would refuse, by the button and by Enter', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const prepared: string[] = [];
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' }, (base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'prepare-channel') prepared.push(action.name);
      return base.chat(store, action, view);
    },
  }));
  await ui.waitFor(() => assert.ok(heads().length));
  const name = await openCreateForm(/^Engineering/);
  const create = () =>
    ui.screen.getByRole<HTMLButtonElement>('button', {
      name: /Create channel/,
    });
  const form = name.closest('form');
  assert.ok(form);
  // Engineering already has a general channel, so an empty name is refused —
  // and the hint that says an empty name creates one does not stand beside it.
  await ui.screen.findByText('This team already has a general channel.');
  assert.equal(ui.screen.queryByText(/An empty name creates/), null);
  assert.equal(create().disabled, true);
  // Enter is the button: it sends exactly what the button would send.
  ui.fireEvent.submit(form);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(prepared, []);
  ui.fireEvent.change(name, { target: { value: 'design' } });
  const description = ui.screen.getByRole('textbox', {
    name: 'Channel description',
  });
  // The description band is the one `ChatLimits` admits, checked rather than
  // merely printed, at both ends.
  ui.fireEvent.change(description, { target: { value: 'ab' } });
  await ui.screen.findByText(
    'Descriptions must be at least 3 characters or empty.',
  );
  assert.equal(create().disabled, true);
  ui.fireEvent.change(description, { target: { value: 'x'.repeat(513) } });
  await ui.screen.findByText('Descriptions are at most 512 characters.');
  assert.equal(create().disabled, true);
  ui.fireEvent.submit(form);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(prepared, []);
  ui.fireEvent.change(description, { target: { value: '' } });
  await ui.waitFor(() => assert.equal(create().disabled, false));
});

test('New chat says what a channel is, and counts only the ones it offers', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' }, (base) =>
    withChannels(base, 'team:household', [
      { id: '11'.repeat(16), name: 'chores', muted: true },
      { id: '22'.repeat(16), name: 'archive', hidden: true },
    ]),
  );
  await ui.waitFor(() =>
    assert.deepEqual(channels(), ['#general', '#chores', '#archive']),
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'New chat' }));
  const household = await ui.screen.findByRole('radio', { name: /^Household/ });
  // Three channels are listed in the picker; the hidden one is not one the
  // count line offers.
  assert.match(household.textContent ?? '', /2 channels · /);
  ui.fireEvent.click(household);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Continue' }));
  // A channel the column draws as hidden or muted says the same thing here.
  const archive = await ui.screen.findByRole('radio', { name: /#archive/ });
  assert.match(archive.textContent ?? '', /Hidden/);
  assert.match(
    ui.screen.getByRole('radio', { name: /#chores/ }).textContent ?? '',
    /Muted/,
  );
});

test('a team switch keeps a New chat whose submission is unresolved', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  let lose = false;
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' }, (base) => ({
    ...base,
    chat: async (store, action, view) => {
      const reply = await base.chat(store, action, view);
      if (action.action === 'prepare-channel' && lose)
        throw {
          code: 'ambiguous',
          message: 'Preparation reply lost',
          ambiguous: true,
          retryable: false,
          fatal: false,
        };
      return reply;
    },
  }));
  await ui.waitFor(() => assert.ok(heads().length));
  // A half-finished New chat belongs to the team it was opened in: a switch
  // closes it rather than rebinding it to the team that arrives.
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'New chat' }));
  await ui.screen.findByRole('dialog', { name: 'New chat' });
  ui.fireEvent.click(head('Household'));
  await ui.waitFor(() => assert.equal(ui.screen.queryByRole('dialog'), null));
  // A submission the agent has already been given is the exception: it has to
  // be settled where it was made, so the switch leaves the sheet standing.
  lose = true;
  const name = await openCreateForm(/^Engineering/);
  ui.fireEvent.change(name, { target: { value: 'design' } });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: /Create channel/ }));
  await ui.screen.findByText('Preparation reply lost');
  ui.fireEvent.click(head('Household'));
  await ui.waitFor(() =>
    assert.equal(head('Household').getAttribute('aria-current'), 'page'),
  );
  assert.ok(ui.screen.getByRole('dialog', { name: 'New channel' }));
  assert.ok(ui.screen.getByRole('button', { name: 'Retry channel creation' }));
});

test('an ambiguous access failure keeps the submission a recovery re-issues', async () => {
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/lease.ts',
  )) as typeof import('../src/model/lease');
  const fresh = await snapshotWithChat(['personal', 'acme']);
  const submissions: string[] = [];
  const shell: Partial<Shell> = {};
  let held!: ReturnType<typeof holdingFirst>;
  await mount(
    fresh,
    { kind: 'chat', ref: 'team:household' },
    (base) => {
      held = holdingFirst(base, 'prepare-channel', (action) => {
        if (action.action === 'prepare-channel')
          submissions.push(action.submission);
      });
      return held.bridge;
    },
    undefined,
    shell,
  );
  await ui.waitFor(() => assert.ok(heads().length));
  const name = await openCreateForm(/^Engineering/);
  ui.fireEvent.change(name, { target: { value: 'design' } });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: /Create channel/ }));
  // The check-in lapses while the preparation is in flight, so the reply lands
  // on a team this Mac can no longer reach: the agent may hold the submission,
  // and a failure that may have been taken is not one to mint a second id for.
  await held.reached;
  await ui.act(async () => {
    shell.setSnapshot?.(applyLease(fresh, 'lapsed', 'acme'));
  });
  held.release();
  await ui.screen.findByText(/Access changed after the channel was sent/);
  // Nothing else in the sheet can be reached while the team is out of reach,
  // so the recovery is not disabled by the team's own state.
  const recover = () =>
    ui.screen.getByRole<HTMLButtonElement>('button', {
      name: 'Retry channel creation',
    });
  assert.equal(recover().disabled, false);
  ui.fireEvent.click(recover());
  await ui.screen.findByText('Access changed before the channel was sent.');
  assert.equal(submissions.length, 1, 'a refused request is not sent');
  // Access comes back, and the recovery re-issues the same submission rather
  // than a second one beside it.
  await ui.act(async () => {
    shell.setSnapshot?.(fresh);
  });
  ui.fireEvent.click(recover());
  await ui.waitFor(() => assert.equal(submissions.length, 2));
  assert.equal(new Set(submissions).size, 1);
});

test('a channel is not attempted once the server’s access generation has moved', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const attempts: string[] = [];
  const shell: Partial<Shell> = {};
  let held!: ReturnType<typeof holdingFirst>;
  await mount(
    snapshot,
    { kind: 'chat', ref: 'team:household' },
    (base) => {
      const counted: Bridge = {
        ...base,
        chat: async (store, action, view) => {
          if (action.action === 'attempt' && store === 'team:eng')
            attempts.push(action.operation);
          return base.chat(store, action, view);
        },
      };
      held = holdingFirst(counted, 'prepare-channel');
      return held.bridge;
    },
    undefined,
    shell,
  );
  await ui.waitFor(() => assert.ok(heads().length));
  const name = await openCreateForm(/^Engineering/);
  ui.fireEvent.change(name, { target: { value: 'design' } });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: /Create channel/ }));
  await held.reached;
  // The shell re-established access on that server while the preparation was
  // in flight. The guard reads the generation the shell is on now, not the one
  // the request started under, so the attempt is not sent on the old footing.
  await ui.act(async () => {
    shell.setGenerations?.(new Map([['acme', 1]]));
  });
  held.release();
  await ui.screen.findByText(/Access changed after the channel was sent/);
  assert.deepEqual(attempts, []);
  await ui.screen.findByRole('button', { name: 'Retry channel creation' });
});

test('a channel preparation under a changed identity stops chat and frees the sheet', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(
    snapshot,
    { kind: 'chat', ref: 'team:eng', channel: 'ab'.repeat(16) },
    (base) => ({
      ...base,
      chat: async (store, action, view) => {
        const reply = await base.chat(store, action, view);
        // A reply under a different identity is not this team's.
        return action.action === 'prepare-channel'
          ? { ...reply, scope: { ...reply.scope, actor: '99'.repeat(16) } }
          : reply;
      },
    }),
  );
  await ui.waitFor(() => assert.ok(heads().length));
  const name = await openCreateForm(/^Engineering/);
  ui.fireEvent.change(name, { target: { value: 'design' } });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: /Create channel/ }));
  const sheet = await ui.screen.findByRole('dialog', { name: 'New channel' });
  await ui.waitFor(() =>
    assert.ok(
      ui.screen.getAllByText('The chat identity changed.').length >= 2,
      'the sheet and the pane both say what happened',
    ),
  );
  // The session the submission was made in is over, so there is nothing left
  // to recover it with: the sheet states the reason and can be dismissed
  // rather than holding the reader on a recovery that cannot run.
  ui.fireEvent.keyDown(sheet, { key: 'Escape' });
  await ui.waitFor(() => assert.ok(!ui.screen.queryByRole('dialog')));
  await ui.screen.findByText('Chat stopped');
});

test('a channel created before its synchronization lands reads as loading', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  let created = false;
  let release = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' }, (base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'sync-inbox' && store === 'team:eng' && created)
        await gate;
      const reply = await base.chat(store, action, view);
      if (action.action === 'attempt' && store === 'team:eng') created = true;
      return reply;
    },
  }));
  await ui.waitFor(() => assert.ok(heads().length));
  const name = await openCreateForm(/^Engineering/);
  ui.fireEvent.change(name, { target: { value: 'design' } });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: /Create channel/ }));
  // The channel exists; the team's next synchronization is what lists it. A
  // pane that said it was unavailable would be calling the reader's own work
  // missing.
  await ui.screen.findByText('Loading Engineering…');
  assert.equal(
    ui.screen.queryByRole('heading', { name: 'Channel unavailable' }),
    null,
  );
  release();
  await ui.screen.findByRole('button', { name: /#design/ });
});

test('a team inbox stays channel-less until a conversation is selected', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const shell: Shell = {
    setSnapshot: () => {},
    setGenerations: () => {},
    accessNow: () => Date.now() / 1000,
  };
  const journal = await mount(
    snapshot,
    { kind: 'chat', ref: 'team:eng' },
    undefined,
    undefined,
    shell,
  );
  await ui.screen.findByText('Choose a channel to open a conversation.');
  await ui.waitFor(() => assert.ok(head('Engineering')));
  assert.equal(journal.length, 0);
  const { crumbTrail } = (await vite.ssrLoadModule(
    '/src/shell/topbar.tsx',
  )) as typeof import('../src/shell/topbar');
  assert.deepEqual(crumbTrail({ kind: 'chat', ref: 'team:eng' }, snapshot), [
    'Chat',
    'Engineering',
  ]);
  await ui.waitFor(() =>
    assert.match(head('Engineering').textContent ?? '', /Engineering/),
  );
  ui.fireEvent.click(head('Engineering'));
  await ui.waitFor(() =>
    assert.equal(
      (journal.at(-1) as Extract<Location, { kind: 'chat' }>)?.channel,
      'ab'.repeat(16),
    ),
  );
  const { parentLocation } = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const parent = parentLocation(journal.at(-1)!);
  assert.ok(parent);
  await ui.act(async () => {
    shell.setLocation?.(parent);
  });
  await ui.screen.findByText('Choose a channel to open a conversation.');
  assert.equal(parentLocation(parent), null);
  assert.equal(journal.length, 1, 'returning to the inbox does not redirect');
});

test('single-channel teams sit under Conversations, multi-channel teams under Teams, and an empty section has no heading', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' });
  // Both teams start with the general channel alone, so only Conversations
  // has anything to head.
  await ui.waitFor(() =>
    assert.deepEqual(
      [...document.querySelectorAll('.sec')].map((node) => node.textContent),
      ['Conversations'],
    ),
  );
  ui.cleanup();
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' }, (base) =>
    withChannels(base, 'team:household', [
      { id: '11'.repeat(16), name: 'chores' },
    ]),
  );
  // Household now heads its own channel list, so both headings draw, in
  // that order, and each team sits under the one its shape calls for.
  await ui.waitFor(() => assert.deepEqual(channels(), ['#general', '#chores']));
  assert.deepEqual(
    [...document.querySelectorAll('.sec')].map((node) => node.textContent),
    ['Conversations', 'Teams'],
  );
  assert.ok(head('Engineering').classList.contains('chat-conv'));
  assert.ok(head('Household').classList.contains('chat-team-head'));
});

test('the collapse control folds a team, keeps its unread total, excludes a muted channel from it, and leaves settings reachable', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const journal = await mount(
    snapshot,
    { kind: 'chat', ref: 'team:eng' },
    (base) =>
      withInbox(
        withChannels(base, 'team:household', [
          { id: '11'.repeat(16), name: 'chores', unread: '3' },
          { id: '22'.repeat(16), name: 'archive', unread: '2', muted: true },
        ]),
        'team:household',
        (inbox) => {
          // Pin the general channel's own count so the heading's rolled-up
          // total is exactly what this test adds, not whatever the fixture's
          // default conversation happens to carry.
          inbox.conversations = inbox.conversations.map((conversation) =>
            conversation.channel.name
              ? conversation
              : { ...conversation, unread: '0' },
          );
        },
      ),
  );
  await ui.waitFor(() =>
    assert.deepEqual(channels(), ['#general', '#chores', '#archive']),
  );
  const twist = ui.screen.getByRole('button', { name: 'Collapse Household' });
  assert.equal(twist.getAttribute('aria-expanded'), 'true');
  ui.fireEvent.click(twist);
  // Folded: the channel rows are gone (Engineering's own general channel is a
  // single-row conversation, not a `.chat-channel`, so nothing else remains),
  // and the heading's own badge carries the total of what folded away — the
  // muted channel's 2 excluded, the same way the rail's own unread count
  // already excludes a muted channel.
  await ui.waitFor(() => assert.deepEqual(channels(), []));
  const expand = ui.screen.getByRole('button', { name: 'Expand Household' });
  assert.equal(expand.getAttribute('aria-expanded'), 'false');
  const badge = head('Household').querySelector('.chat-unread');
  assert.equal(badge?.textContent, '3');
  assert.equal(badge?.getAttribute('aria-label'), '3 unread');
  // The gear beside the new control still opens team settings.
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Team settings for Household' }),
  );
  assert.deepEqual(journal.at(-1), {
    kind: 'group-settings',
    ref: 'team:household',
    tab: 'settings',
  });
  // Expanding restores the channel list exactly as it was.
  ui.fireEvent.click(expand);
  await ui.waitFor(() =>
    assert.deepEqual(channels(), ['#general', '#chores', '#archive']),
  );
});

test('a search overrides a fold so a channel it matched inside a collapsed team is seen', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' }, (base) =>
    withChannels(base, 'team:household', [
      { id: '11'.repeat(16), name: 'chores' },
    ]),
  );
  await ui.waitFor(() => assert.deepEqual(channels(), ['#general', '#chores']));
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Collapse Household' }),
  );
  await ui.waitFor(() => assert.deepEqual(channels(), []));
  const field = ui.screen.getByRole('searchbox', {
    name: 'Search teams and channels',
  });
  ui.fireEvent.change(field, { target: { value: 'chores' } });
  await ui.waitFor(() => assert.deepEqual(channels(), ['#chores']));
  // Clearing the query brings the fold back, unchanged underneath.
  ui.fireEvent.change(field, { target: { value: '' } });
  await ui.waitFor(() => assert.deepEqual(channels(), []));
});

test('the conversation header states the member count from the roster the team page already loads', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const engineers = snapshot.parties.filter((p) => p.store === 'team:eng');
  assert.ok(engineers.length > 0, 'the fixture seeds Engineering with parties');
  await mount(snapshot, {
    kind: 'chat',
    ref: 'team:eng',
    channel: 'ab'.repeat(16),
  });
  await ui.waitFor(() =>
    assert.match(
      document.querySelector('.chat-thread-title h2')?.textContent ?? '',
      /Engineering/,
    ),
  );
  const count = await ui.waitFor(() => {
    const node = document.querySelector('.chat-member-count');
    assert.ok(node, 'the header carries a member count');
    return node;
  });
  assert.equal(count.textContent, `${engineers.length} members`);
});

test('the header omits the member count while the open team’s roster has not arrived', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const withoutRoster = {
    ...snapshot,
    parties: snapshot.parties.filter((party) => party.store !== 'team:eng'),
  };
  await mount(withoutRoster, {
    kind: 'chat',
    ref: 'team:eng',
    channel: 'ab'.repeat(16),
  });
  await ui.waitFor(() =>
    assert.match(
      document.querySelector('.chat-thread-title h2')?.textContent ?? '',
      /Engineering/,
    ),
  );
  // The count is the whole subtitle, so a roster that has not arrived leaves
  // the line empty rather than showing a false zero.
  assert.equal(document.querySelector('.chat-member-count'), null);
  assert.equal(document.querySelector('.chat-thread-title p')?.textContent, '');
});
