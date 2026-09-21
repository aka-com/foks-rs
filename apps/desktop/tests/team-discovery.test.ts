import assert from 'node:assert/strict';
import test from 'node:test';
import {
  accountHasBoundTeam,
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

test('an account is bound to a team only by a team store of its own', () => {
  const snapshot = fixture();
  const bound = snapshot.accounts.find((account) =>
    accountHasBoundTeam(snapshot, account),
  );
  assert.ok(bound);
  // The catalog holds no team for this account, so discovery has everything
  // still to bind for it and keeps its own interval.
  assert.equal(
    accountHasBoundTeam(
      { ...snapshot, stores: snapshot.stores.filter((s) => s.kind !== 'team') },
      bound,
    ),
    false,
  );
  // Another account's teams, and another server's, are not this account's.
  assert.equal(
    accountHasBoundTeam(snapshot, { ...bound, alias: 'someone-else' }),
    false,
  );
  assert.equal(
    accountHasBoundTeam(snapshot, { ...bound, server: 'another.server' }),
    false,
  );
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

test('only a discovery that wrote a binding is followed by a catalog read', async () => {
  const account = discoveryAccounts(fixture())[0];
  const group = {
    alias: 'discovered',
    accountAlias: account.alias,
    teamIdHex: `03${'ab'.repeat(32)}`,
    kind: 'named' as const,
    name: 'Discovered',
    active: true,
  };
  const reads = async (
    reply: Awaited<ReturnType<Bridge['discoverGroups']>>,
  ): Promise<number> => {
    let count = 0;
    await reconcileTeamDiscovery(
      { discoverGroups: async () => reply } as unknown as Bridge,
      account,
      context(),
      () => true,
      async () => {
        count++;
      },
    );
    return count;
  };
  // Nothing was written locally, so nothing changed for the catalog to show.
  assert.equal(
    await reads({ accountAlias: account.alias, groups: [group], bound: [] }),
    0,
  );
  // A binding was written: the profile is read back so the team appears.
  assert.equal(
    await reads({
      accountAlias: account.alias,
      groups: [group],
      bound: [group.alias],
    }),
    1,
  );
  // An agent that reports no bound list says nothing about what it wrote,
  // which keeps the read it always made.
  assert.equal(
    await reads({ accountAlias: account.alias, groups: [group] }),
    1,
  );
});
