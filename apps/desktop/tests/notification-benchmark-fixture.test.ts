import assert from 'node:assert/strict';
import test from 'node:test';
import { notificationBenchmarkSnapshot } from '../../../scripts/benchmarks/chat-notification-fixture';
import { chatAvailable } from '../src/model';

test('notification acceptance fixture satisfies the current access model without bypassing its gates', () => {
  const scope = {
    host: 'host',
    actor: 'actor',
    store: {
      profile: 'receiver',
      account_alias: 'receiver',
      team_alias: 'bench',
      team_id: 'team',
    },
  };
  const snapshot = notificationBenchmarkSnapshot('store', scope);
  const store = snapshot.stores[0];
  assert.equal(chatAvailable(snapshot, store), true);
  assert.equal(
    chatAvailable(
      { ...snapshot, agent: { state: 'bootstrap', step: 'unlock' } },
      store,
    ),
    false,
  );
  const denied = structuredClone(snapshot);
  denied.servers[0].services.chat = false;
  assert.equal(chatAvailable(denied, store), false);
});
