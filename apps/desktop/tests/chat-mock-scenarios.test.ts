import assert from 'node:assert/strict';
import test from 'node:test';
import { mockChat } from '../src/chat-mock';
import {
  CHAT_MOCK_SCENARIOS,
  chatMockScenario,
  MOCK_CHAT_ACTOR,
  MOCK_CHAT_ARCHIVE,
  MOCK_CHAT_DEPARTED,
  MOCK_CHAT_DEPLOYS,
  MOCK_CHAT_GENERAL,
  MOCK_CHAT_HOST,
  MOCK_CHAT_INCIDENTS,
  MOCK_CHAT_PREPARED,
  MOCK_CHAT_RANDOM,
  MOCK_CHAT_REJECTED,
  MOCK_CHAT_REJECTION_CODE,
  MOCK_CHAT_UNCERTAIN,
  type ChatMockScenario,
} from '../src/chat-mock-scenarios';
import {
  decodeChatReply,
  type ChatAction,
  type ChatReply,
} from '../src/chat-contract';
import { FIXTURE } from '../src/fixture';

// The native decoder checks the scope against a JSON store id, so the
// Engineering team is renamed to one; its roster follows it.
const eng = FIXTURE.stores.find((s) => s.id === 'team:eng');
if (eng?.kind !== 'team') throw new Error('fixture lacks Engineering');
const storeId = JSON.stringify({
  kind: 'team',
  profile: eng.server,
  accountAlias: eng.account,
  teamAlias: eng.alias,
  teamId: eng.team_id_hex,
});
const snapshot = {
  ...FIXTURE,
  stores: FIXTURE.stores.map((s) => (s === eng ? { ...s, id: storeId } : s)),
  parties: FIXTURE.parties.map((p) =>
    p.store === eng.id ? { ...p, store: storeId } : p,
  ),
};
const owner = FIXTURE.parties.find(
  (p) => p.store === eng.id && p.username === 'sam.ortiz',
)!.party_id_hex;
const roster = new Set(
  FIXTURE.parties.filter((p) => p.store === eng.id).map((p) => p.party_id_hex),
);

/** A mock chat whose every reply must pass the native decoder. */
function decoded(scenario: ChatMockScenario) {
  const chat = mockChat(snapshot, scenario);
  return async (action: ChatAction): Promise<ChatReply> =>
    decodeChatReply(await chat(storeId, action), storeId, action);
}

async function history(
  chat: (action: ChatAction) => Promise<ChatReply>,
  channel: string,
  before: string | null = null,
) {
  const reply = await chat({ action: 'history', channel, before });
  if (reply.result.kind !== 'history') throw new Error('history expected');
  return reply.result;
}

async function inbox(chat: (action: ChatAction) => Promise<ChatReply>) {
  const reply = await chat({ action: 'sync-inbox', blocked_channels: [] });
  if (reply.result.kind !== 'inbox') throw new Error('inbox expected');
  return reply.result;
}

test('the chat-scenario parameter names a scenario, else the default', () => {
  assert.equal(chatMockScenario('?chat-scenario=busy'), 'busy');
  assert.equal(
    chatMockScenario('?state=chat&chat-scenario=integrity'),
    'integrity',
  );
  assert.equal(chatMockScenario('?chat-scenario=other'), 'default');
  assert.equal(chatMockScenario(''), 'default');
});

for (const scenario of CHAT_MOCK_SCENARIOS) {
  test(`${scenario}: every history row has a sender and every reply decodes`, async () => {
    const chat = decoded(scenario);
    const { conversations, channels } = await inbox(chat);
    assert.equal(conversations.length, channels.length);
    for (const { channel } of conversations) {
      let before: string | null = null;
      do {
        const page = await history(chat, channel.id, before);
        for (const m of page.messages) assert.notEqual(m.sender, null);
        before = page.before;
      } while (before !== null);
    }
    await chat({ action: 'pending' });
  });
}

test('default: the seed message is from the team Owner', async () => {
  const chat = decoded('default');
  const page = await history(chat, MOCK_CHAT_GENERAL);
  assert.deepEqual(
    page.messages.map((m) => m.sender),
    [owner],
  );
  // Without another member, the only member sent it.
  const bare = await mockChat()('team', {
    action: 'history',
    channel: MOCK_CHAT_GENERAL,
    before: null,
  });
  if (bare.result.kind !== 'history') throw new Error('history expected');
  assert.equal(bare.result.messages[0].sender, MOCK_CHAT_ACTOR);
  // The agent advances the read pointer over the local account's own send.
  const bareInbox = await mockChat()('team', {
    action: 'sync-inbox',
    blocked_channels: [],
  });
  if (bareInbox.result.kind !== 'inbox') throw new Error('inbox expected');
  assert.equal(bareInbox.result.conversations[0].unread, '0');
});

test('busy: four channels, a long unread run and a departed sender', async () => {
  const chat = decoded('busy');
  const { conversations } = await inbox(chat);
  assert.deepEqual(
    conversations.map((c) => [
      c.channel.name,
      c.channel.admin,
      c.read_through,
      c.unread,
    ]),
    [
      ['', false, '78', '42'],
      ['deploys', false, '6', '0'],
      ['incidents', true, '2', '2'],
      ['random', false, '5', '0'],
    ],
  );
  const senders = new Set<string | null>();
  let before: string | null = null;
  let count = 0;
  let grouped = 0;
  do {
    const page = await history(chat, MOCK_CHAT_GENERAL, before);
    page.messages.forEach((m, i) => {
      senders.add(m.sender);
      // The agent advances the read pointer over the local account's sends.
      if (BigInt(m.sequence) > 78n) assert.notEqual(m.sender, MOCK_CHAT_ACTOR);
      if (i && page.messages[i - 1].sender === m.sender) grouped += 1;
    });
    count += page.messages.length;
    before = page.before;
  } while (before !== null);
  assert.equal(count, 120);
  assert.ok(grouped > 0);
  assert.ok(senders.has(MOCK_CHAT_ACTOR));
  assert.ok(senders.has(MOCK_CHAT_DEPARTED));
  assert.ok(!roster.has(MOCK_CHAT_DEPARTED));
  for (const sender of senders)
    if (sender !== MOCK_CHAT_ACTOR && sender !== MOCK_CHAT_DEPARTED)
      assert.ok(roster.has(sender!));
});

test('failures: unsent and unconfirmed operations, a rejection and an intent load error', async () => {
  const chat = decoded('failures');
  const pending = await chat({ action: 'pending' });
  if (pending.result.kind !== 'pending') throw new Error('pending expected');
  assert.deepEqual(
    pending.result.operations.map((op) => [op.id, op.state]),
    [
      [MOCK_CHAT_PREPARED, 'prepared'],
      [MOCK_CHAT_UNCERTAIN, 'uncertain'],
      [MOCK_CHAT_REJECTED, 'prepared'],
    ],
  );
  const body = await chat({
    action: 'operation-body',
    operation: MOCK_CHAT_UNCERTAIN,
    channel: MOCK_CHAT_GENERAL,
  });
  assert.equal(
    body.result.kind === 'operation-body' && body.result.text,
    'Can someone confirm the 2.4 tag?',
  );
  // Checking delivery finds no message, so the operation stays unconfirmed.
  const checked = await chat({
    action: 'reconcile',
    operation: MOCK_CHAT_UNCERTAIN,
  });
  assert.equal(
    checked.result.kind === 'operation' && checked.result.operation.state,
    'uncertain',
  );
  // The attempt fails, and the ledger then reports the host's code.
  await assert.rejects(
    chat({ action: 'attempt', operation: MOCK_CHAT_REJECTED }),
    { code: 'operation-failed' },
  );
  const rejected = await chat({
    action: 'status',
    operation: MOCK_CHAT_REJECTED,
  });
  if (rejected.result.kind !== 'operation')
    throw new Error('operation expected');
  assert.equal(rejected.result.operation.state, 'rejected');
  assert.equal(
    rejected.result.operation.rejection_code,
    MOCK_CHAT_REJECTION_CODE,
  );
  const sent = await chat({ action: 'attempt', operation: MOCK_CHAT_PREPARED });
  assert.equal(
    sent.result.kind === 'operation' && sent.result.operation.state,
    'confirmed',
  );
  const target = {
    host: MOCK_CHAT_HOST,
    actor: MOCK_CHAT_ACTOR,
  };
  await chat({
    action: 'load-intent',
    ...target,
    channel: MOCK_CHAT_GENERAL,
  });
  await assert.rejects(
    chat({ action: 'load-intent', ...target, channel: MOCK_CHAT_DEPLOYS }),
    { code: 'operation-failed' },
  );
});

test('integrity: missing predecessors, a degraded inbox, and stopped, muted and hidden channels', async () => {
  const chat = decoded('integrity');
  const result = await inbox(chat);
  assert.equal(result.degraded, true);
  assert.ok(BigInt(result.cursor) < BigInt(result.head));
  assert.deepEqual(result.blocked_channels, [MOCK_CHAT_INCIDENTS]);
  const byId = new Map(result.conversations.map((c) => [c.channel.id, c]));
  assert.equal(byId.get(MOCK_CHAT_INCIDENTS)?.preview, null);
  assert.equal(byId.get(MOCK_CHAT_RANDOM)?.muted, true);
  assert.equal(byId.get(MOCK_CHAT_RANDOM)?.unread, '3');
  assert.equal(byId.get(MOCK_CHAT_ARCHIVE)?.hidden, true);
  assert.deepEqual(
    [...byId.values()]
      .filter((c) => c.muted || c.hidden)
      .map((c) => c.channel.id),
    [MOCK_CHAT_RANDOM, MOCK_CHAT_ARCHIVE],
  );
  const page = await history(chat, MOCK_CHAT_DEPLOYS);
  assert.equal(page.messages[0].sequence, '3');
  assert.deepEqual(page.missing_predecessors, ['1', '2']);
  assert.deepEqual(
    (await history(chat, MOCK_CHAT_GENERAL)).missing_predecessors,
    [],
  );
  // A message sent after the gap takes the next sequence.
  const sent = await chat({
    action: 'submit-message',
    submission: 'ef'.repeat(16),
    channel: MOCK_CHAT_DEPLOYS,
    text: 'after the gap',
  });
  assert.equal(
    sent.result.kind === 'operation' &&
      sent.result.operation.confirmation?.kind === 'message-sent' &&
      sent.result.operation.confirmation.sequence,
    '9',
  );
});

test('a scripted session on a fixture store ID decodes as the agent would reply', async () => {
  // The fixture ID is not a native JSON store reference; mock replies are
  // decoded against the team it stands for.
  const chat = mockChat(FIXTURE);
  const call = (action: ChatAction) => chat('team:eng', action);
  const text = (reply: ChatReply) =>
    reply.result.kind === 'history'
      ? reply.result.messages.map((m) =>
          m.content.kind === 'text' ? m.content.text : m.content.kind,
        )
      : [];
  const first = await call({ action: 'sync-inbox', blocked_channels: [] });
  assert.equal(first.scope.store.team_alias, eng.alias);
  assert.equal(first.scope.store.team_id, eng.team_id_hex);
  if (first.result.kind !== 'inbox') throw new Error('inbox expected');
  // A long line of four-byte characters: the inbox preview is cut on a
  // character boundary within the snippet bound, and notification history
  // to 256 characters, as the agent cuts them.
  const long = '\u{1F600}'.repeat(300);
  for (const [index, body] of [long, 'x'.repeat(300)].entries())
    await call({
      action: 'submit-message',
      submission: String(index + 1).repeat(32),
      channel: MOCK_CHAT_GENERAL,
      text: body,
    });
  assert.deepEqual(
    text(
      await call({
        action: 'notification-history',
        channel: MOCK_CHAT_GENERAL,
        before: null,
      }),
    ).slice(-2),
    ['\u{1F600}'.repeat(256), 'x'.repeat(256)],
  );
  assert.deepEqual(
    text(
      await call({
        action: 'history',
        channel: MOCK_CHAT_GENERAL,
        before: null,
      }),
    ).slice(-2),
    [long, 'x'.repeat(300)],
  );
  await call({
    action: 'submit-message',
    submission: '3'.repeat(32),
    channel: MOCK_CHAT_GENERAL,
    text: `${long}\nsecond line`,
  });
  const second = await call({ action: 'sync-inbox', blocked_channels: [] });
  if (second.result.kind !== 'inbox') throw new Error('inbox expected');
  const preview = second.result.conversations[0].preview?.content;
  assert.deepEqual(preview, {
    kind: 'text',
    text: `${'\u{1F600}'.repeat(127)}…`,
  });
  const incremental = await call({
    action: 'history',
    channel: MOCK_CHAT_GENERAL,
    before: null,
    after: '2',
  });
  assert.equal(
    incremental.result.kind === 'history' && incremental.result.gap,
    false,
  );
  assert.equal(
    (
      await call({
        action: 'poll-inbox',
        since: first.result.head,
        timeout_milliseconds: 0,
      })
    ).result.kind,
    'poll',
  );
  assert.equal(
    (
      await call({
        action: 'mark-read',
        channel: MOCK_CHAT_GENERAL,
        sequence: '4',
      })
    ).result.kind,
    'read',
  );
  const created = await call({
    action: 'prepare-channel',
    submission: 'c'.repeat(32),
    name: 'deploys',
    description: 'Deploy notes.',
    admin: false,
  });
  if (created.result.kind !== 'operation')
    throw new Error('operation expected');
  const id = created.result.operation.id;
  await call({ action: 'attempt', operation: id });
  const channels = await call({ action: 'channels' });
  assert.equal(
    channels.result.kind === 'channels' && channels.result.channels.length,
    2,
  );
  const pending = await call({
    action: 'prepare-message',
    submission: 'd'.repeat(32),
    channel: MOCK_CHAT_GENERAL,
    text: 'held',
  });
  if (pending.result.kind !== 'operation')
    throw new Error('operation expected');
  // A reply is a decoded copy: changing it does not change the mock's ledger.
  pending.result.operation.state = 'cancelled';
  const listed = await call({ action: 'pending' });
  assert.deepEqual(
    listed.result.kind === 'pending' &&
      listed.result.operations.map((op) => op.state),
    ['prepared'],
  );
  await call({ action: 'cancel', operation: pending.result.operation.id });
  assert.equal(
    (await call({ action: 'cleanup-pending' })).result.kind,
    'cleanup-pending',
  );
});
