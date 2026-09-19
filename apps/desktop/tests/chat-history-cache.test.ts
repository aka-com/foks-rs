import assert from 'node:assert/strict';
import test from 'node:test';
import { ChatHistoryCache } from '../src/chat/history-cache';
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
    cache.get(a);
    cache.accept(c, page('c', '1', 'éé'), null);
    assert.ok(cache.get(a));
    assert.equal(cache.get(b), null);
    assert.ok(cache.get(c));
  }
});

test('a full retained window can advance to the newest page without remaining stuck at its limit', () => {
  const cache = new ChatHistoryCache();
  cache.update('t', inbox(), 0);
  const binding = cache.binding('t', 'a', 0)!;
  const full = page();
  full.messages = Array.from(
    { length: 1000 },
    (_, index) => page('a', String(index + 1)).messages[0],
  );
  cache.accept(binding, full, null);
  cache.accept(binding, page('a', '1001'), null);
  assert.deepEqual(
    cache.get(binding)?.messages.map((message) => message.sequence),
    ['1001'],
  );
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
