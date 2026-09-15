import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeChatReply, sequence } from '../src/chat-contract';
import {
  decodeLocation,
  encodeLocation,
  transition,
  INITIAL_STATE,
} from '../src/location';
const store = {
  profile: 'local',
  account_alias: 'me',
  team_alias: 'team',
  team_id: '03' + 'ab'.repeat(32),
};
const storeId = JSON.stringify({
  kind: 'team',
  profile: store.profile,
  accountAlias: store.account_alias,
  teamAlias: store.team_alias,
  teamId: store.team_id,
});
const channel = 'ab'.repeat(16);
test('chat navigation round trips and clears vault-only state', () => {
  const location = { kind: 'chat' as const, ref: storeId, channel };
  const encoded = encodeLocation(location);
  const params = new URLSearchParams({ state: encoded.state });
  for (const [k, v] of Object.entries(encoded.params))
    if (v !== null) params.set(k, v);
  assert.deepEqual(decodeLocation(params.toString()), location);
  assert.equal(
    transition(
      { ...INITIAL_STATE, query: 'secret query' },
      { type: 'navigate', location },
    ).query,
    '',
  );
  assert.equal(
    encodeLocation({ kind: 'store', ref: storeId }).params.channel,
    null,
  );
});
test('chat decoding preserves large sequences and rejects wrong identity and contradictory rows', () => {
  const message = {
    id: 'cd'.repeat(16),
    sequence: '9007199254740993',
    sender: null,
    send_time: '1700000000000',
    insert_time: '1700000000001',
    content: { kind: 'text', text: '<script>hostile</script>' },
  };
  const reply = {
    scope: {
      store,
      host: '02' + 'ab'.repeat(32),
      actor: '01' + 'ab'.repeat(32),
    },
    result: {
      kind: 'history',
      channel,
      messages: [message],
      before: message.sequence,
      missing_predecessors: [],
    },
  };
  const action = { action: 'history' as const, channel, before: null };
  assert.equal(decodeChatReply(reply, storeId, action).result.kind, 'history');
  assert.equal(sequence(message.sequence), message.sequence);
  assert.throws(() =>
    decodeChatReply(
      {
        ...reply,
        scope: { ...reply.scope, store: { ...store, account_alias: 'other' } },
      },
      storeId,
      action,
    ),
  );
  assert.throws(() =>
    decodeChatReply(
      { ...reply, result: { ...reply.result, messages: [message, message] } },
      storeId,
      action,
    ),
  );
  assert.throws(() =>
    decodeChatReply(
      { ...reply, result: { ...reply.result, before: '2' } },
      storeId,
      action,
    ),
  );
  assert.throws(() => sequence('01'));
  assert.throws(() => sequence('9223372036854775808'));
});

test('inbox, read, and poll results retain exact decimal state', () => {
  const scope = {
    store,
    host: '02' + 'ab'.repeat(32),
    actor: '01' + 'ab'.repeat(32),
  };
  const conversation = {
    channel: {
      id: channel,
      name: '',
      description: 'Team updates',
      admin: false,
      readable: true,
      writable: true,
      read_role: 'Member (0)',
      write_role: 'Member (0)',
    },
    inbox_version: '9007199254740993',
    read_through: '1',
    pending_read: null,
    unread: '9007199254740992',
    hidden: false,
    muted: false,
    preview: {
      sender: '01' + 'cd'.repeat(32),
      send_time: '1700000000000',
      insert_time: '1700000000001',
      content: { kind: 'text', text: 'Latest update' },
    },
  };
  const inbox = {
    scope,
    result: {
      kind: 'inbox',
      channels: [],
      read_retry_pending: false,
      previews_incomplete: false,
      blocked_channels: [],
      cursor: '9007199254740993',
      head: '9007199254740993',
      degraded: false,
      conversations: [conversation],
    },
  };
  const decoded = decodeChatReply(inbox, storeId, { action: 'sync-inbox' });
  assert.equal(decoded.result.kind, 'inbox');
  assert.throws(() =>
    decodeChatReply(
      {
        ...inbox,
        result: { ...inbox.result, head: '9007199254740994' },
      },
      storeId,
      { action: 'sync-inbox' },
    ),
  );
  assert.throws(() =>
    decodeChatReply(
      {
        ...inbox,
        result: {
          ...inbox.result,
          conversations: [conversation, conversation],
        },
      },
      storeId,
      { action: 'sync-inbox' },
    ),
  );
  assert.equal(
    decodeChatReply(
      {
        scope,
        result: {
          kind: 'poll',
          bumped: true,
          inbox_version: '9007199254740994',
        },
      },
      storeId,
      {
        action: 'poll-inbox',
        since: '9007199254740993',
        timeout_milliseconds: 25_000,
      },
    ).result.kind,
    'poll',
  );
  assert.equal(
    decodeChatReply(
      { scope, result: { kind: 'read', channel, sequence: '7' } },
      storeId,
      { action: 'mark-read', channel, sequence: '7' },
    ).result.kind,
    'read',
  );
});

test('shared Rust and TypeScript chat reply fixtures agree', async () => {
  const { readFile } = await import('node:fs/promises');
  const fixtures = JSON.parse(
    await readFile(
      new URL(
        '../../../crates/foks-agent-proto/tests/fixtures/chat-replies.json',
        import.meta.url,
      ),
      'utf8',
    ),
  ) as {
    store: typeof store;
    cases: {
      name: string;
      valid: boolean;
      action: import('../src/chat-contract').ChatAction;
      reply: unknown;
    }[];
  };
  const id = JSON.stringify({
    kind: 'team',
    profile: fixtures.store.profile,
    accountAlias: fixtures.store.account_alias,
    teamAlias: fixtures.store.team_alias,
    teamId: fixtures.store.team_id,
  });
  for (const fixture of fixtures.cases) {
    if (fixture.valid)
      assert.doesNotThrow(
        () => decodeChatReply(fixture.reply, id, fixture.action),
        fixture.name,
      );
    else
      assert.throws(
        () => decodeChatReply(fixture.reply, id, fixture.action),
        fixture.name,
      );
  }
});

test('notification history accepts only bounded snippets and retains history scope validation', () => {
  const reply = {
    scope: {
      store,
      host: '02' + 'ab'.repeat(32),
      actor: '01' + 'ab'.repeat(32),
    },
    result: {
      kind: 'history',
      channel,
      before: null,
      missing_predecessors: [],
      messages: [
        {
          id: 'cd'.repeat(16),
          sequence: '1',
          sender: null,
          send_time: '1',
          insert_time: '1',
          content: { kind: 'text', text: '😀'.repeat(256) },
        },
      ],
    },
  };
  const action = {
    action: 'notification-history' as const,
    channel,
    before: null,
  };
  assert.equal(decodeChatReply(reply, storeId, action).result.kind, 'history');
  reply.result.messages[0].content.text += 'x';
  assert.throws(() => decodeChatReply(reply, storeId, action));
});
