import assert from 'node:assert/strict';
import test from 'node:test';
import {
  ARRIVAL_ANNOUNCE_MS,
  ARRIVAL_CLEAR_MS,
  ArrivalAnnouncer,
  arrivalText,
  channelArrivals,
} from '../src/chat/arrival-announcer';
import type { ChatClock } from '../src/chat/clock';
import type { TeamInbox } from '../src/chat/inbox-service';
import type { ChatConversation } from '../src/chat-contract';

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
  advance(ms: number) {
    const end = this.time + ms;
    for (;;) {
      const next = [...this.tasks].sort((a, b) => a[1].due - b[1].due)[0];
      if (!next || next[1].due > end) break;
      this.time = next[1].due;
      this.tasks.delete(next[0]);
      next[1].fn();
    }
    this.time = end;
  }
}

const ME = 'aa'.repeat(16);
const PRIYA = 'bb'.repeat(16);
const NAMES = new Map([[PRIYA, 'priya.n']]);

/** A conversation at `position`, everything past `read` unread. */
function conversation(
  id: string,
  name: string,
  position: number,
  options: {
    read?: number;
    sender?: string | null;
    kind?: 'text' | 'unverifiable';
    muted?: boolean;
    hidden?: boolean;
    readable?: boolean;
    preview?: boolean;
  } = {},
): ChatConversation {
  const read = options.read ?? 0;
  return {
    channel: {
      id,
      name,
      description: null,
      admin: false,
      readable: options.readable ?? true,
      writable: true,
      read_role: 'member',
      write_role: 'member',
    },
    inbox_version: String(position),
    read_through: String(read),
    pending_read: null,
    unread: String(position - read),
    hidden: options.hidden ?? false,
    muted: options.muted ?? false,
    preview:
      options.preview === false
        ? null
        : {
            sender: options.sender === undefined ? PRIYA : options.sender,
            send_time: '1700000000000',
            insert_time: '1700000000001',
            content:
              options.kind === 'unverifiable'
                ? { kind: 'unverifiable' }
                : { kind: 'text', text: 'the message body' },
          },
  };
}

function team(
  conversations: ChatConversation[],
  blocked: string[] = [],
): TeamInbox {
  return {
    state: 'ready',
    scope: {
      store: {
        profile: 'p',
        account_alias: 'a',
        team_alias: 't',
        team_id: '00'.repeat(16),
      },
      host: 'h',
      actor: ME,
    },
    data: {
      kind: 'inbox',
      channels: conversations.map((c) => c.channel),
      read_retry_pending: false,
      previews_incomplete: false,
      blocked_channels: blocked,
      cursor: '0',
      head: '0',
      degraded: false,
      conversations,
    },
    error: '',
    note: '',
    stale: false,
    revision: 1,
    channelRevisions: new Map(),
    blockedChannels: new Set(blocked),
  };
}

const names = () => NAMES;

test('arrivals are counted from the last stated position, never on first sight', () => {
  const first = channelArrivals(
    new Map(),
    new Map([['team:a', team([conversation('01', '', 4)])]]),
    names,
  );
  assert.deepEqual(first.arrivals, []);
  const second = channelArrivals(
    first.positions,
    new Map([
      [
        'team:a',
        team([conversation('01', '', 6), conversation('02', 'deploys', 1)]),
      ],
    ]),
    names,
  );
  // #deploys is new to the inbox: its position is set, not announced.
  assert.deepEqual(second.arrivals, [
    {
      team: 'team:a',
      channel: '01',
      title: '#general',
      count: 2,
      sender: 'priya.n',
    },
  ]);
  // Reading moves the read pointer and the unread count together; the
  // position stays, so nothing arrived.
  const read = channelArrivals(
    second.positions,
    new Map([
      [
        'team:a',
        team([
          conversation('01', '', 6, { read: 6 }),
          conversation('02', 'deploys', 1),
        ]),
      ],
    ]),
    names,
  );
  assert.deepEqual(read.arrivals, []);
  // A team whose inbox has no data keeps its positions.
  const empty: TeamInbox = { ...team([]), data: undefined };
  const kept = channelArrivals(
    read.positions,
    new Map([['team:a', empty]]),
    names,
  );
  assert.equal(kept.positions.get('team:a')?.get('01'), 6n);
});

test('own, muted, hidden, unreadable and stopped channels are not announced', () => {
  const before = (conversations: ChatConversation[], blocked?: string[]) =>
    channelArrivals(
      new Map(),
      new Map([['team:a', team(conversations, blocked)]]),
      names,
    ).positions;
  const after = (
    conversations: ChatConversation[],
    later: ChatConversation[],
    blocked?: string[],
  ) =>
    channelArrivals(
      before(conversations, blocked),
      new Map([['team:a', team(later, blocked)]]),
      names,
    ).arrivals;
  const general = (position: number, options = {}) =>
    conversation('01', '', position, options);
  assert.deepEqual(after([general(1)], [general(2, { sender: ME })]), []);
  assert.deepEqual(after([general(1)], [general(2, { muted: true })]), []);
  assert.deepEqual(after([general(1)], [general(2, { hidden: true })]), []);
  assert.deepEqual(after([general(1)], [general(2, { readable: false })]), []);
  assert.deepEqual(after([general(1)], [general(2)], ['01']), []);
  // Without a preview, only a channel counting unread messages is reported,
  // and its sender is not named.
  assert.deepEqual(
    after([general(1)], [general(2, { preview: false, read: 2 })]),
    [],
  );
  assert.deepEqual(after([general(1)], [general(2, { preview: false })]), [
    { team: 'team:a', channel: '01', title: '#general', count: 1 },
  ]);
  // The sender of a message that could not be verified is a claim, so it is
  // not named.
  assert.deepEqual(
    after([general(1)], [general(2, { kind: 'unverifiable' })]),
    [{ team: 'team:a', channel: '01', title: '#general', count: 1 }],
  );
});

test('the announcement names sender and channel and never the text', () => {
  const one = {
    team: 'team:a',
    channel: '01',
    title: '#general',
    count: 1,
    sender: 'priya.n',
  };
  const noTeam = () => undefined;
  assert.equal(
    arrivalText([one], noTeam),
    'New message from priya.n in #general',
  );
  assert.equal(
    arrivalText([{ ...one, count: 3 }], noTeam),
    '3 new messages in #general, latest from priya.n',
  );
  assert.equal(
    arrivalText([{ ...one, sender: undefined }], () => 'Household'),
    'New message in #general in Household',
  );
  const channel = (title: string) => ({ ...one, channel: title, title });
  assert.equal(
    arrivalText([channel('#a'), channel('#b')], noTeam),
    'New messages in #a and #b',
  );
  assert.equal(
    arrivalText([channel('#a'), channel('#b'), channel('#c')], noTeam),
    'New messages in #a, #b and #c',
  );
  assert.equal(
    arrivalText(
      [channel('#a'), channel('#b'), channel('#c'), channel('#d')],
      noTeam,
    ),
    'New messages in #a, #b, #c and 1 more channel',
  );
});

test('announcements are rate-limited, merged, cleared, and need focus', () => {
  const clock = new Clock();
  const said: string[] = [];
  let focused = true;
  const announcer = new ArrivalAnnouncer(
    clock,
    (text) => said.push(text),
    () => focused,
  );
  const inbox = (general: number, deploys: number) =>
    new Map([
      [
        'team:a',
        team([
          conversation('01', '', general),
          conversation('02', 'deploys', deploys),
        ]),
      ],
    ]);
  const observe = (general: number, deploys: number) =>
    announcer.observe(inbox(general, deploys), names, () => undefined);
  observe(1, 1);
  assert.deepEqual(said, []);
  // The first arrival is announced at once.
  observe(2, 1);
  assert.deepEqual(said, ['New message from priya.n in #general']);
  // Arrivals within the interval wait for its end and are merged.
  clock.advance(1_000);
  observe(3, 1);
  clock.advance(1_000);
  observe(4, 2);
  assert.equal(said.length, 1);
  // The region is cleared before the next announcement, so the same words
  // are announced again.
  clock.advance(ARRIVAL_CLEAR_MS - 2_000);
  assert.deepEqual(said.slice(1), ['']);
  clock.advance(ARRIVAL_ANNOUNCE_MS);
  assert.deepEqual(said.slice(2), [
    'New messages in #general and #deploys',
    '',
  ]);
  // Arrivals while the window is not focused are not announced, then or
  // later.
  focused = false;
  clock.advance(ARRIVAL_ANNOUNCE_MS);
  observe(5, 2);
  focused = true;
  clock.advance(ARRIVAL_ANNOUNCE_MS);
  assert.deepEqual(said.slice(4), []);
  // An arrival that waits for the interval is dropped if focus leaves first.
  observe(6, 2);
  assert.deepEqual(said.slice(4), ['New message from priya.n in #general']);
  observe(7, 2);
  focused = false;
  clock.advance(ARRIVAL_ANNOUNCE_MS);
  assert.deepEqual(said.slice(5), ['']);
  // Stopping cancels what waits, and the clearing.
  focused = true;
  observe(8, 2);
  observe(9, 2);
  announcer.stop();
  clock.advance(ARRIVAL_ANNOUNCE_MS);
  assert.deepEqual(said.slice(6), ['New message from priya.n in #general']);
});
