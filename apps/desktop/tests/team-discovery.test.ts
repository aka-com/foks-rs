import assert from 'node:assert/strict';
import test from 'node:test';
import {
  discoveryAccounts,
  reconcileTeamDiscovery,
} from '../src/team-discovery';
import type { Bridge } from '../src/bridge';
import { FIXTURE } from '../src/fixture';
import type { AgentSnapshot } from '../src/model';

function fixture(): AgentSnapshot {
  return {
    ...FIXTURE,
    agent: { state: 'ready' },
    servers: FIXTURE.servers.map((server) => ({
      ...server,
      trust: { status: 'verified' },
      compatibility: { status: 'not-required' },
      passiveStatus: { status: 'available', source: 'signed-server-status' },
      restrictions: [],
    })),
    profileInventory: FIXTURE.servers.map((server) => ({
      profile: server.id,
      accounts: 'complete',
      teams: 'complete',
    })),
  };
}
const context = () => ({
  signal: new AbortController().signal,
  trigger: 'periodic' as const,
  isCurrent: () => true,
});

test('recurring discovery includes accounts already bound to teams and does not require KV permission', () => {
  const snapshot = fixture();
  const accounts = discoveryAccounts(snapshot);
  assert.ok(accounts.length > 0);
  const bound = accounts.find((account) =>
    snapshot.stores.some(
      (store) =>
        store.kind === 'team' &&
        store.server === account.server &&
        store.account === account.alias,
    ),
  );
  assert.ok(bound);
  const restricted = {
    ...snapshot,
    servers: snapshot.servers.map((server) => ({
      ...server,
      compatibility: {
        status: 'required' as const,
        expiresAt: 100_000,
        capabilities: ['teams' as const],
      },
    })),
    storeInventory: snapshot.storeInventory.map((entry) => ({
      ...entry,
      status: 'unavailable' as const,
    })),
  };
  assert.ok(
    discoveryAccounts(restricted, 1).some(
      (account) => account.store === bound.store,
    ),
  );
  assert.equal(discoveryAccounts(restricted, 100_001).length, 0);
});

test('discovery reconciles after success or partial failure without replaying the mutation', async () => {
  const account = discoveryAccounts(fixture())[0];
  for (const failed of [false, true]) {
    let calls = 0,
      reads = 0;
    const error = Object.assign(new Error('interrupted'), {
      code: 'ambiguous',
      ambiguous: true,
    });
    const bridge = {
      discoverGroups: async () => {
        calls++;
        if (failed) throw error;
        return { accountAlias: account.alias, groups: [] };
      },
    } as unknown as Bridge;
    const pending = reconcileTeamDiscovery(
      bridge,
      account,
      context(),
      () => true,
      async () => {
        reads++;
      },
    );
    if (failed) await assert.rejects(pending, (cause) => cause === error);
    else await pending;
    assert.equal(calls, 1);
    assert.equal(reads, 1);
  }
});

test('retired or denied discovery never dispatches or modifies known team state', async () => {
  const snapshot = fixture(),
    account = discoveryAccounts(snapshot)[0];
  const bridge = {
    discoverGroups: () => assert.fail('must not dispatch'),
  } as unknown as Bridge;
  await assert.rejects(
    reconcileTeamDiscovery(
      bridge,
      account,
      context(),
      () => false,
      async () => assert.fail('nothing dispatched'),
    ),
    { code: 'cancelled' },
  );
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(
    reconcileTeamDiscovery(
      bridge,
      account,
      { ...context(), signal: controller.signal, isCurrent: () => false },
      () => true,
      async () => assert.fail('retired'),
    ),
    { code: 'cancelled' },
  );
  assert.deepEqual(snapshot.stores, FIXTURE.stores);
});
