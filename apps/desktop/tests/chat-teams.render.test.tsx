/**
 * The Chat tab's team column: which teams it lists, which one is expanded,
 * what a collapsed team carries, and what the tab shows when no team has chat.
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

async function mount(
  snapshot: AgentSnapshot,
  start: Location,
  override?: (bridge: Bridge) => Bridge,
  clock?: ChatClock,
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
    return createElement(
      StrictMode,
      null,
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot: overlayRoot,
        children: createElement(ChatInboxProvider, {
          bridge,
          snapshot,
          clock,
          children: createElement(ChatTab, {
            snapshot,
            bridge,
            location,
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

function heads(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('.chat-team-head')];
}

function head(name: string): HTMLElement {
  const row = heads().find(
    (node) => node.querySelector('b')?.textContent === name,
  );
  assert.ok(row, `${name} is in the column`);
  return row;
}

test('the tab with no team opens the first team that has chat', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  const journal = await mount(snapshot, { kind: 'chat' });
  await ui.waitFor(() => assert.ok(heads().length));
  // The first team in the fixture's navigation order is the one that opens.
  assert.deepEqual(journal.at(0), { kind: 'chat', ref: 'team:eng' });
  // The row selects a team rather than folding one away, so the open team is
  // the current one, not an expanded disclosure.
  assert.equal(head('Engineering').getAttribute('aria-current'), 'true');
  assert.equal(head('Engineering').getAttribute('aria-expanded'), null);
  assert.equal(head('Household').getAttribute('aria-current'), null);
});

test('expanding a team selects it and collapses the one that was open', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  // Everything the mounted conversation asks of a team, as opposed to the
  // inbox polling the column reads for every team.
  const conversation: string[] = [];
  const clock = new Clock();
  const journal = await mount(
    snapshot,
    { kind: 'chat', ref: 'team:household' },
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
  await ui.waitFor(() => assert.ok(heads().length));
  assert.equal(head('Engineering').getAttribute('aria-current'), null);
  const column = document.querySelector('.chat-inbox');
  await ui.waitFor(() =>
    assert.ok(conversation.some((call) => call.startsWith('team:household'))),
  );
  ui.fireEvent.click(head('Engineering'));
  assert.deepEqual(journal.at(-1), { kind: 'chat', ref: 'team:eng' });
  await ui.waitFor(() =>
    assert.equal(head('Engineering').getAttribute('aria-current'), 'true'),
  );
  assert.equal(head('Household').getAttribute('aria-current'), null);
  // The column is the tab's, not the conversation's: the switch replaced the
  // conversation beside it without rebuilding it.
  assert.equal(document.querySelector('.chat-inbox'), column);
  // Only the open team lists channels: they all sit under its heading. The
  // channels come from the inbox service, which runs on the test's clock.
  await clock.advance(1000);
  await ui.waitFor(() =>
    assert.ok(document.querySelectorAll('.chat-channel').length),
  );
  const channels = [...document.querySelectorAll('.chat-channel')];
  const expanded = head('Engineering').closest('.chat-team');
  assert.ok(channels.every((row) => expanded?.contains(row)));
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

test('a collapsed team carries its own unread count and channel total', async () => {
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
  // Household is collapsed: no conversation is mounted for it, and the count
  // comes from the inbox service the rail already reads. The badge sits inside
  // the row that names the team, so its label is the count alone.
  const badge = await ui.waitFor(() => {
    const node = head('Household').querySelector('.chat-unread');
    assert.ok(node, 'Household carries an unread badge');
    return node;
  });
  assert.equal(badge.getAttribute('aria-label'), '2 unread');
  assert.equal(badge.textContent, '2');
  await ui.waitFor(() =>
    assert.match(
      head('Household').querySelector('small')?.textContent ?? '',
      /channel/,
    ),
  );
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
    engineering.querySelector('small')?.textContent ?? '',
    /^Chat not offered on /,
  );
  const labels = [...document.querySelectorAll('.sec')].map(
    (node) => node.textContent,
  );
  assert.ok(labels.includes('Teams'));
  assert.ok(labels.includes('No chat'));
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
  assert.equal(engineering.getAttribute('aria-current'), 'true');
  assert.equal(
    engineering.querySelector('small')?.textContent,
    'foks.acme-corp.com',
  );
  assert.equal(
    engineering.querySelector('.chat-unread')?.getAttribute('aria-label'),
    'Check-in expired',
  );
  assert.equal(engineering.querySelector('.chat-unread')?.textContent, '!');
  // A locked team cannot be opened: the pane says so, and any channel row the
  // inbox does list is disabled rather than merely absent for a moment.
  await ui.screen.findByRole('heading', { name: 'Engineering chat is locked' });
  assert.ok(
    [...document.querySelectorAll<HTMLButtonElement>('.chat-channel')].every(
      (row) => row.disabled,
    ),
  );
  // Collapsed, the summary is the same fact, not an invented instruction.
  ui.fireEvent.click(head('Household'));
  await ui.waitFor(() =>
    assert.equal(
      head('Engineering').querySelector('small')?.textContent,
      'Check-in expired',
    ),
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

test('with no team at all the tab says how chat gets turned on', async () => {
  const snapshot = await snapshotWithChat([]);
  const journal = await mount(snapshot, { kind: 'chat' });
  await ui.screen.findByRole('heading', { name: 'No team chats yet' });
  await ui.screen.findByText('How chat gets turned on');
  assert.ok(ui.screen.getByText('No team on this Mac has chat.'));
  // Nothing is selected, so nothing is navigated to.
  assert.deepEqual(journal, []);
  // Creating or joining a team is the Teams tab.
  ui.fireEvent.click(
    ui.screen.getAllByRole('button', { name: 'Create or join a team' })[0],
  );
  assert.deepEqual(journal.at(-1), { kind: 'teams' });
});

test('the note names the team the tab chose and leaves focus in the pane', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  await mount(snapshot, { kind: 'chat' });
  await ui.screen.findByText(
    'No team was named, so Chat opened Engineering, the first team with chat on this Mac.',
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Dismiss' }));
  await ui.waitFor(() =>
    assert.equal(ui.screen.queryByText(/No team was named/), null),
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
  await ui.screen.findByText(/No team was named/);
  // An explicit selection removes the automatic-selection note permanently.
  ui.fireEvent.click(head('Household'));
  await ui.waitFor(() =>
    assert.equal(head('Household').getAttribute('aria-current'), 'true'),
  );
  assert.equal(ui.screen.queryByText(/No team was named/), null);
  ui.fireEvent.click(head('Engineering'));
  await ui.waitFor(() =>
    assert.equal(head('Engineering').getAttribute('aria-current'), 'true'),
  );
  assert.equal(ui.screen.queryByText(/No team was named/), null);
});

test('unfinished work sits in a bounded section and keeps the composer', async () => {
  const snapshot = await snapshotWithChat(['personal', 'acme']);
  let bridge: Bridge | undefined;
  await mount(snapshot, { kind: 'chat', ref: 'team:eng' }, (base) => {
    bridge = base;
    return base;
  });
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
