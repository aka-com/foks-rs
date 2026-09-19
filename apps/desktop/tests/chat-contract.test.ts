import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeChatReply, sequence } from '../src/chat-contract';
import {
  CHAT_DESCRIPTION_MAX_CHARS,
  CHAT_DESCRIPTION_MIN_CHARS,
  CHAT_NAME_MAX_CHARS,
  CHAT_NAME_MIN_CHARS,
} from '../src/chat-limits';
import {
  channelDescriptionProblem,
  channelNameProblem,
  normalizeChannelDescription,
  normalizeChannelName,
} from '../src/chat/presentation';
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

test('submit operation replies bind the message kind and channel strictly', () => {
  const action = {
    action: 'submit-message' as const,
    submission: 'ef'.repeat(16),
    channel,
    text: 'body',
  };
  const operation = {
    id: 'cd'.repeat(16),
    channel,
    kind: 'send-message',
    state: 'confirmed',
    receipt: { kind: 'message-sent', sequence: '2' },
    rejection_code: null,
  };
  const reply = {
    scope: {
      store,
      host: '02' + 'ab'.repeat(32),
      actor: '01' + 'ab'.repeat(32),
    },
    result: { kind: 'operation', operation },
  };
  assert.deepEqual(decodeChatReply(reply, storeId, action), reply);
  for (const changed of [
    { channel: 'ef'.repeat(16) },
    { kind: 'create-channel', receipt: { kind: 'channel-created' } },
  ])
    assert.throws(() =>
      decodeChatReply(
        {
          ...reply,
          result: {
            kind: 'operation',
            operation: { ...operation, ...changed },
          },
        },
        storeId,
        action,
      ),
    );
  assert.throws(() =>
    decodeChatReply(
      { ...reply, result: { kind: 'pending', operations: [] } },
      storeId,
      action,
    ),
  );
});
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

test('the channel name and description bands are the ones the agent admits', () => {
  // The numbers are not typed twice: they are the shared policy the Rust build
  // compiles its constants from, and `foks-agent` asserts those against
  // `ChatLimits`, which is what the client actually admits a channel on.
  assert.equal(CHAT_NAME_MIN_CHARS, 3);
  assert.equal(CHAT_NAME_MAX_CHARS, 32);
  assert.equal(CHAT_DESCRIPTION_MIN_CHARS, 3);
  assert.equal(CHAT_DESCRIPTION_MAX_CHARS, 512);
  assert.equal(channelNameProblem('a'.repeat(CHAT_NAME_MAX_CHARS)), null);
  assert.equal(
    channelNameProblem('a'.repeat(CHAT_NAME_MAX_CHARS + 1)),
    `Channel names are at most ${CHAT_NAME_MAX_CHARS} characters.`,
  );
  assert.equal(
    channelNameProblem('a'.repeat(CHAT_NAME_MIN_CHARS - 1)),
    `Channel names are at least ${CHAT_NAME_MIN_CHARS} characters.`,
  );
  assert.equal(
    channelDescriptionProblem('x'.repeat(CHAT_DESCRIPTION_MAX_CHARS)),
    null,
  );
  assert.equal(
    channelDescriptionProblem('x'.repeat(CHAT_DESCRIPTION_MAX_CHARS + 1)),
    `Descriptions are at most ${CHAT_DESCRIPTION_MAX_CHARS} characters.`,
  );
  assert.equal(
    channelDescriptionProblem('xx'),
    `Descriptions must be at least ${CHAT_DESCRIPTION_MIN_CHARS} characters or empty.`,
  );
  assert.equal(channelDescriptionProblem(''), null);
});

test('general is an alias for the unnamed channel and still detects duplicates', () => {
  assert.equal(normalizeChannelName('  GENERAL  '), '');
  assert.equal(channelNameProblem('general'), null);
  assert.equal(channelNameProblem('GeNeRaL', ['design']), null);
  assert.equal(
    channelNameProblem('general', ['']),
    'This team already has a general channel.',
  );
  assert.equal(
    channelNameProblem('', ['']),
    'This team already has a general channel.',
  );
});

test('lowercasing follows the agent, which keeps one scalar per character', () => {
  // The agent lowercases per character and keeps the first scalar of the
  // mapping, so a name at the bound stays at the bound. JavaScript's own
  // `toLowerCase()` expands "İ" into two scalars, which would count a name of
  // 32 characters as 33 and refuse a name the agent takes.
  assert.equal(normalizeChannelName('İ'), 'i');
  assert.equal([...normalizeChannelName('İ'.repeat(32))].length, 32);
  assert.equal(channelNameProblem('İ'.repeat(CHAT_NAME_MAX_CHARS)), null);
  assert.equal([...normalizeChannelDescription('İ'.repeat(512))].length, 512);
  assert.equal(
    channelDescriptionProblem('İ'.repeat(CHAT_DESCRIPTION_MAX_CHARS)),
    null,
  );
  // Ordinary lowercasing and trimming are unchanged.
  assert.equal(normalizeChannelName('  Design  '), 'design');
  assert.equal(normalizeChannelDescription('Team DECISIONS'), 'team decisions');
});
