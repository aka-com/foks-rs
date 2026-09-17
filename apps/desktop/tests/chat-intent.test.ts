import assert from 'node:assert/strict';
import test from 'node:test';
import { chatIntentPersistence } from '../src/chat/intent';
import { decodeLocalSession } from '../src/chat/local-contract';
import type { LocalSession } from '../src/chat/local-contract';
import type { ChatScope } from '../src/chat-contract';

const scope: ChatScope = {
  host: '02' + 'ab'.repeat(32),
  actor: '01' + 'ab'.repeat(32),
  store: {
    profile: 'local',
    account_alias: 'owner',
    team_alias: 'team',
    team_id: '03' + 'cd'.repeat(32),
  },
};
const storeId = JSON.stringify({
  kind: 'team',
  profile: scope.store.profile,
  accountAlias: scope.store.account_alias,
  teamAlias: scope.store.team_alias,
  teamId: scope.store.team_id,
});
const channel = 'ab'.repeat(16);
const saved = {
  storeId,
  scope,
  channel,
  submission: 'cd'.repeat(16),
  text: 'saved before network',
};
const session: LocalSession = {
  epoch: 'ab'.repeat(16),
  available: true,
  settings: { enabled: false, previews: false, overrides: {} },
};

test('saved message decoding validates the complete identity and bounded text', () => {
  assert.deepEqual(
    decodeLocalSession({ ...session, intent: saved }).intent,
    saved,
  );
  for (const bad of [
    { ...saved, text: '' },
    { ...saved, text: 'x'.repeat(65537) },
    { ...saved, submission: 'bad' },
    { ...saved, channel: 'bad' },
    {
      ...saved,
      scope: {
        ...scope,
        store: { ...scope.store, account_alias: 'replacement' },
      },
    },
  ])
    assert.throws(() => decodeLocalSession({ ...session, intent: bad }));
});

test('local intent replies cannot cross account/channel identities or change saved input', async () => {
  const service = (intent = saved) =>
    chatIntentPersistence(
      { chatLocal: async () => ({ ...session, intent }) },
      storeId,
      scope,
      channel,
    );
  assert.deepEqual(await service().load(), {
    submission: saved.submission,
    text: saved.text,
  });
  await service().save(saved);
  for (const other of [
    { ...saved, storeId: 'other' },
    { ...saved, channel: 'ef'.repeat(16) },
    { ...saved, scope: { ...scope, actor: '01' + 'ef'.repeat(32) } },
  ])
    await assert.rejects(service(other).load(), { code: 'chat-intent' });
  await assert.rejects(service({ ...saved, text: 'different' }).save(saved), {
    code: 'chat-intent',
  });
  await assert.rejects(
    service({ ...saved, submission: 'ef'.repeat(16) }).save(saved),
    { code: 'chat-intent' },
  );
  const absent = chatIntentPersistence(
    { chatLocal: async () => session },
    storeId,
    scope,
    channel,
  );
  assert.equal(await absent.load(), undefined);
  await assert.rejects(absent.save(saved), { code: 'chat-intent' });
});

test('save and clear use the same immutable submission and full scope', async () => {
  const calls: unknown[] = [];
  const persistence = chatIntentPersistence(
    {
      chatLocal: async (action) => {
        calls.push(action);
        return { ...session, intent: saved };
      },
    },
    storeId,
    scope,
    channel,
  );
  await persistence.save(saved);
  await persistence.clear(saved.submission);
  assert.deepEqual(calls, [
    { action: 'save-intent', ...saved },
    {
      action: 'clear-intent',
      storeId,
      scope,
      channel,
      submission: saved.submission,
    },
  ]);
});
