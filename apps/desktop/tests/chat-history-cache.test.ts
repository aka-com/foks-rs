import assert from 'node:assert/strict';
import test from 'node:test';
import { ChatHistoryCache } from '../src/chat/history-cache';
import { freezeDto, readonlySet } from '../src/chat/snapshots';
import type { TeamInbox } from '../src/chat/inbox-service';
import type { ChatResult } from '../src/chat-contract';

function inbox(): TeamInbox {
  return {
    state: 'ready',
    error: '',
    note: '',
    stale: false,
    revision: 0,
    channelRevisions: new Map(),
    blockedChannels: new Set(),
    scope: {
      host: 'host',
      actor: 'actor',
      store: {
        profile: 'p',
        account_alias: 'a',
        team_alias: 't',
        team_id: 'team',
      },
    },
    data: {
      kind: 'inbox',
      cursor: '1',
      head: '1',
      degraded: false,
      read_retry_pending: false,
      previews_incomplete: false,
      blocked_channels: [],
      channels: ['a', 'b', 'c'].map((id) => ({
        id,
        name: id,
        description: null,
        admin: false,
        readable: true,
        writable: true,
        read_role: 'Member (0)',
        write_role: 'Member (0)',
      })),
      conversations: [],
    },
  };
}
function page(
  channel = 'a',
  sequence = '1',
  text = 'hello',
): Extract<ChatResult, { kind: 'history' }> {
  return {
    kind: 'history',
    channel,
    before: null,
    missing_predecessors: [],
    messages: [
      {
        id: channel + sequence,
        sequence,
        sender: 'actor',
        send_time: '1',
        insert_time: '1',
        content: { kind: 'text', text },
      },
    ],
  };
}

test('history is retained per channel, merged, and immutable after publication', () => {
  const cache = new ChatHistoryCache();
  cache.update('t', inbox(), 0);
  const a = cache.binding('t', 'a', 0)!;
  const b = cache.binding('t', 'b', 0)!;
  const first = page();
  cache.accept(a, first, null);
  cache.accept(b, page('b'), null);
  first.messages[0].content = { kind: 'text', text: 'modified' };
  assert.equal(cache.get(a)?.messages[0].content.kind, 'text');
  assert.deepEqual(cache.get(a)?.messages[0].content, {
    kind: 'text',
    text: 'hello',
  });
  cache.accept(a, page('a', '2'), null);
  assert.deepEqual(
    cache.get(a)?.messages.map((m) => m.sequence),
    ['1', '2'],
  );
  assert.equal(cache.get(b)?.messages.length, 1);
  assert.throws(() => {
    cache.get(a)!.messages.length = 0;
  });
  assert.equal('set' in cache.get(a)!.verification, false);
});

for (const change of [
  'actor',
  'host',
  'account',
  'team',
  'profile',
  'generation',
  'blocked',
  'unavailable',
  'removed',
  'readable',
  'role',
  'channel-blocked',
  'clear',
] as const) {
  test(`history and in-flight acceptance are retired after ${change} changes`, () => {
    const cache = new ChatHistoryCache();
    const next = inbox();
    cache.update('t', next, 0);
    const binding = cache.binding('t', 'a', 0)!;
    cache.accept(binding, page(), null);
    let generation = 0;
    if (change === 'actor') next.scope!.actor = 'other';
    if (change === 'host') next.scope!.host = 'other';
    if (change === 'account') next.scope!.store.account_alias = 'other';
    if (change === 'team') next.scope!.store.team_id = 'other';
    if (change === 'profile') next.scope!.store.profile = 'other';
    if (change === 'generation') generation++;
    if (change === 'blocked' || change === 'unavailable') next.state = change;
    if (change === 'removed') next.data!.channels = [];
    if (change === 'readable') next.data!.channels[0].readable = false;
    if (change === 'role') next.data!.channels[0].read_role = 'Admin (0)';
    if (change === 'channel-blocked') next.blockedChannels = new Set(['a']);
    if (change === 'clear') cache.clear();
    cache.update('t', next, generation);
    assert.equal(cache.get(binding), null);
    assert.throws(() => cache.accept(binding, page('a', '2'), null), {
      code: 'cancelled',
    });
    cache.update('t', inbox(), generation);
    assert.equal(cache.get(cache.binding('t', 'a', generation)), null);
  });
}

test('ordinary inbox refresh retains history; channel quarantine leaves siblings intact', () => {
  const cache = new ChatHistoryCache();
  const next = inbox();
  cache.update('t', next, 0);
  const a = cache.binding('t', 'a', 0)!;
  const b = cache.binding('t', 'b', 0)!;
  cache.accept(a, page(), null);
  cache.accept(b, page('b'), null);
  cache.update('t', inbox(), 0);
  assert.equal(cache.binding('t', 'a', 0), a);
  next.blockedChannels = new Set(['a']);
  cache.update('t', next, 0);
  assert.equal(cache.get(a), null);
  assert.equal(cache.get(b)?.messages.length, 1);
});

test('retained history has aggregate channel, row, and UTF-8 byte bounds', () => {
  for (const limits of [
    { channels: 2, rows: 10, bytes: 100 },
    { channels: 10, rows: 2, bytes: 100 },
    { channels: 10, rows: 10, bytes: 8 },
  ]) {
    const cache = new ChatHistoryCache(limits);
    cache.update('t', inbox(), 0);
    const a = cache.binding('t', 'a', 0)!;
    const b = cache.binding('t', 'b', 0)!;
    const c = cache.binding('t', 'c', 0)!;
    cache.accept(a, page('a', '1', 'éé'), null);
    cache.accept(b, page('b', '1', 'éé'), null);
    cache.touch(a);
    cache.accept(c, page('c', '1', 'éé'), null);
    assert.ok(cache.get(a));
    assert.equal(cache.get(b), null);
    assert.ok(cache.get(c));
  }
});

test('reading retained history leaves the eviction order unchanged', () => {
  // Rendering reads the open channel's window; only an open or an accepted
  // page records a use.
  const cache = new ChatHistoryCache({ channels: 2, rows: 10, bytes: 100 });
  cache.update('t', inbox(), 0);
  const a = cache.binding('t', 'a', 0)!;
  const b = cache.binding('t', 'b', 0)!;
  const c = cache.binding('t', 'c', 0)!;
  cache.accept(a, page('a'), null);
  cache.accept(b, page('b'), null);
  let published = 0;
  cache.subscribe(() => published++);
  assert.ok(cache.get(a));
  assert.equal(published, 0);
  cache.accept(c, page('c'), null);
  assert.equal(published, 1);
  assert.equal(cache.get(a), null);
  assert.ok(cache.get(b));
  assert.ok(cache.get(c));
  // Recency is not observable, so recording a use publishes nothing.
  cache.touch(b);
  assert.equal(published, 1);
});

/** A page of channel `a` holding sequences `from` through `to`. */
function range(from: number, to: number, text = 'hello') {
  const result = page('a', String(from), text);
  result.messages = Array.from(
    { length: to - from + 1 },
    (_, index) => page('a', String(from + index), text).messages[0],
  );
  result.before = from > 1 ? String(from) : null;
  return result;
}

test('tail growth past the window target keeps the newest rows and pages back from them', () => {
  const cache = new ChatHistoryCache();
  cache.update('t', inbox(), 0);
  const binding = cache.binding('t', 'a', 0)!;
  const flagged = range(551, 600);
  flagged.missing_predecessors = ['1'];
  cache.accept(binding, flagged, null);
  for (let end = 550; end > 200; end -= 50)
    cache.accept(binding, range(end - 49, end), String(end + 1));
  assert.equal(cache.get(binding)?.messages.length, 400);
  assert.equal(cache.get(binding)?.detached, false);
  // One new row arrives: the oldest row leaves, and the older cursor names
  // the oldest row kept, so paging back reads the row that left.
  cache.accept(
    binding,
    { ...range(601, 601), before: '601', gap: false },
    null,
  );
  const held = sequences(cache, binding);
  assert.equal(held.length, 400);
  assert.equal(held[0], '202');
  assert.equal(held.at(-1), '601');
  assert.equal(cache.get(binding)?.before, '202');
  assert.equal(cache.get(binding)?.detached, false);
  assert.equal(unverified(cache, binding), true);
  // A newest page trims the oldest rows with their verification entries.
  cache.accept(binding, range(552, 651), null);
  assert.equal(sequences(cache, binding)[0], '252');
  assert.equal(cache.get(binding)?.verification.size, 400);
});

test('paging older past the window target keeps a contiguous window and detaches it', () => {
  const cache = new ChatHistoryCache();
  cache.update('t', inbox(), 0);
  const binding = cache.binding('t', 'a', 0)!;
  cache.accept(binding, range(951, 1000), null);
  for (let end = 950; end > 500; end -= 50)
    cache.accept(binding, range(end - 49, end), String(end + 1));
  const held = sequences(cache, binding);
  assert.equal(held.length, 400);
  // The newest rows left; the rows that remain run without a break from the
  // last older page.
  assert.equal(held[0], '501');
  assert.equal(held.at(-1), '900');
  assert.deepEqual(
    held,
    Array.from({ length: 400 }, (_, index) => String(501 + index)),
  );
  assert.equal(cache.get(binding)?.before, '501');
  assert.equal(cache.get(binding)?.detached, true);
  // A newer page past the held tail extends it again and trims the oldest
  // end instead.
  cache.accept(binding, range(901, 950), '951', { newer: true });
  assert.equal(sequences(cache, binding)[0], '551');
  assert.equal(sequences(cache, binding).at(-1), '950');
  assert.equal(cache.get(binding)?.before, '551');
});

test('the window target bounds UTF-8 bytes, and never drops the page it accepts', () => {
  const cache = new ChatHistoryCache({
    channels: 16,
    rows: 1000,
    bytes: 1 << 20,
    window: { rows: 400, bytes: 8 },
  });
  cache.update('t', inbox(), 0);
  const binding = cache.binding('t', 'a', 0)!;
  // Each row is four bytes, so a page of three is over the target and kept
  // whole.
  cache.accept(binding, range(4, 6, 'éé'), null);
  assert.deepEqual(sequences(cache, binding), ['4', '5', '6']);
  cache.accept(binding, { ...range(7, 7, 'éé'), gap: false }, null);
  assert.deepEqual(sequences(cache, binding), ['6', '7']);
  assert.equal(cache.get(binding)?.before, '6');
  cache.accept(binding, range(5, 5, 'éé'), '6');
  assert.deepEqual(sequences(cache, binding), ['5', '6']);
  assert.equal(cache.get(binding)?.detached, true);
});

test("one channel's long scrollback leaves the other channels' windows retained", () => {
  const cache = new ChatHistoryCache();
  cache.update('t', inbox(), 0);
  const a = cache.binding('t', 'a', 0)!;
  const b = cache.binding('t', 'b', 0)!;
  const c = cache.binding('t', 'c', 0)!;
  cache.accept(b, page('b'), null);
  cache.accept(c, page('c'), null);
  cache.accept(a, range(1951, 2000), null);
  for (let end = 1950; end > 0; end -= 50)
    cache.accept(a, range(end - 49, end), String(end + 1));
  assert.equal(cache.get(a)?.messages.length, 400);
  assert.ok(cache.get(b));
  assert.ok(cache.get(c));
});

test('a list position belongs to its retained window', () => {
  const cache = new ChatHistoryCache({ channels: 1, rows: 1000, bytes: 1000 });
  cache.update('t', inbox(), 0);
  const a = cache.binding('t', 'a', 0)!;
  const b = cache.binding('t', 'b', 0)!;
  // Nothing is retained for the channel, so there is nothing to place.
  cache.setPosition(a, { message: 'a1', offset: 12 });
  assert.equal(cache.position(a), null);
  cache.accept(a, page('a'), null);
  let published = 0;
  cache.subscribe(() => published++);
  cache.setPosition(a, { message: 'a1', offset: 12 });
  assert.equal(published, 0);
  cache.accept(a, page('a', '2'), null);
  assert.deepEqual(cache.position(a), { message: 'a1', offset: 12 });
  cache.setPosition(a, null);
  assert.equal(cache.position(a), null);
  cache.setPosition(a, { message: 'a2', offset: 0 });
  // Evicting the window drops the position with it.
  cache.accept(b, page('b'), null);
  cache.accept(a, page('a'), null);
  assert.equal(cache.position(a), null);
});

test('conflicting history is rejected without replacing accepted content', () => {
  const cache = new ChatHistoryCache();
  cache.update('t', inbox(), 0);
  const binding = cache.binding('t', 'a', 0)!;
  cache.accept(binding, page(), null);
  assert.throws(() => cache.accept(binding, page('a', '1', 'changed'), null), {
    code: 'chat-channel-integrity',
  });
  assert.deepEqual(cache.get(binding)?.messages[0].content, {
    kind: 'text',
    text: 'hello',
  });
});

/** The sequences a window holds, in order. */
function sequences(
  cache: ChatHistoryCache,
  binding: Parameters<ChatHistoryCache['get']>[0],
): string[] {
  return (cache.get(binding)?.messages ?? []).map(
    (message) => message.sequence,
  );
}

/** True while any retained row is still flagged as unverified. */
function unverified(
  cache: ChatHistoryCache,
  binding: Parameters<ChatHistoryCache['get']>[0],
): boolean {
  return [...(cache.get(binding)?.verification.values() ?? [])].some(Boolean);
}

test('a newest page disjoint from the window replaces it, and older pagination continues from the new page', () => {
  const cache = new ChatHistoryCache();
  cache.update('t', inbox(), 0);
  const binding = cache.binding('t', 'a', 0)!;
  const first = page();
  first.messages = Array.from(
    { length: 50 },
    (_, index) => page('a', String(index + 51)).messages[0],
  );
  first.before = '51';
  cache.accept(binding, first, null);
  // A gap this window cannot bridge: the newest page starts well past its last
  // row, so the window it does not join is dropped rather than merged.
  const newest = page();
  newest.messages = Array.from(
    { length: 50 },
    (_, index) => page('a', String(index + 151)).messages[0],
  );
  newest.before = '151';
  cache.accept(binding, newest, null);
  assert.deepEqual(sequences(cache, binding).at(0), '151');
  assert.deepEqual(sequences(cache, binding).at(-1), '200');
  // Paging back reaches the gap from the page that is held, not from the one
  // that was dropped.
  assert.equal(cache.get(binding)?.before, '151');
});

test('verification warnings survive unrelated pages until the flagged rows are rechecked', () => {
  const cache = new ChatHistoryCache();
  cache.update('t', inbox(), 0);
  const binding = cache.binding('t', 'a', 0)!;
  const flagged = page('a', '3');
  flagged.missing_predecessors = ['1'];
  cache.accept(binding, flagged, null);
  assert.equal(unverified(cache, binding), true);
  // A later page says nothing about the rows already held, so their warning
  // stands.
  cache.accept(binding, page('a', '4', 'next'), null);
  assert.equal(unverified(cache, binding), true);
  const rechecked = page('a', '3');
  rechecked.messages = [
    page('a', '3').messages[0],
    page('a', '4', 'next').messages[0],
  ];
  cache.accept(binding, rechecked, null);
  assert.equal(unverified(cache, binding), false);
});

test('incremental acceptance deduplicates sequences and gap replacement discards the old window', () => {
  const cache = new ChatHistoryCache();
  cache.update('t', inbox(), 0);
  const binding = cache.binding('t', 'a', 0)!;
  cache.accept(binding, page('a', '1'), null);
  const delta = { ...page('a', '2'), before: '2', gap: false };
  cache.accept(binding, delta, null);
  cache.accept(binding, delta, null);
  assert.deepEqual(
    cache.get(binding)?.messages.map((m) => m.sequence),
    ['1', '2'],
  );
  assert.equal(cache.get(binding)?.before, null);
  cache.accept(binding, page('a', '1', 'reset'), null, { replace: true });
  assert.deepEqual(
    cache.get(binding)?.messages.map((m) => m.sequence),
    ['1'],
  );
  assert.equal(cache.get(binding)?.verification.size, 1);
  cache.accept(binding, { ...page(), messages: [] }, null, { replace: true });
  assert.equal(cache.get(binding)?.messages.length, 0);
});

test('owned unchanged authority inputs skip iteration, but clearing and generation changes rebuild', () => {
  const next = inbox();
  let visits = 0;
  const channels = next.data!.channels;
  const iterate = channels[Symbol.iterator].bind(channels);
  channels[Symbol.iterator] = () => {
    visits++;
    return iterate();
  };
  freezeDto(next.data!);
  freezeDto(next.scope!);
  next.blockedChannels = readonlySet([]);
  const cache = new ChatHistoryCache();
  cache.update('t', next, 0);
  assert.equal(visits, 1);
  const binding = cache.binding('t', 'a', 0)!;
  cache.accept(binding, page(), null);
  cache.update(
    't',
    { ...next, revision: 1, channelRefreshRevisions: new Map([['a', 2]]) },
    0,
  );
  assert.equal(visits, 1);
  assert.ok(cache.get(binding));
  cache.update('t', next, 1);
  assert.equal(visits, 2);
  assert.equal(cache.get(binding), null);
  cache.clear('t');
  cache.update('t', next, 1);
  assert.equal(visits, 3);
});

for (const change of [
  'role',
  'quarantine',
  'scope',
  'generation',
  'unavailable',
  'blocked',
] as const) {
  test(`immutable history inputs still retire authority after ${change}`, () => {
    const cache = new ChatHistoryCache();
    const first = inbox();
    freezeDto(first.data!);
    freezeDto(first.scope!);
    first.blockedChannels = readonlySet([]);
    cache.update('t', first, 0);
    const binding = cache.binding('t', 'a', 0)!;
    cache.accept(binding, page(), null);
    let next = { ...first };
    if (change === 'role') {
      next = {
        ...next,
        data: freezeDto({
          ...first.data!,
          channels: first.data!.channels.map((c) => ({
            ...c,
            read_role: 'Admin (0)',
          })),
        }),
      };
    }
    if (change === 'quarantine') next.blockedChannels = readonlySet(['a']);
    if (change === 'scope')
      next.scope = freezeDto({ ...first.scope!, actor: 'other' });
    if (change === 'unavailable' || change === 'blocked') next.state = change;
    cache.update('t', next, Number(change === 'generation'));
    assert.equal(cache.get(binding), null);
    assert.throws(() => cache.accept(binding, page(), null), {
      code: 'cancelled',
    });
  });
}
