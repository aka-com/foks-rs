import assert from 'node:assert/strict';
import test from 'node:test';
import { FIXTURE } from '../src/fixture';
import {
  authoritativeSetupFacts,
  resolveSetupEntry,
} from '../src/first-run-controller';
import { initialFirstRun, transitionFirstRun } from '../src/first-run-state';
const profile = {
  profile: 'personal',
  acceptance: 'unchanged' as const,
  lookupName: 'localhost',
  canonicalName: 'localhost',
  hostId: `02${'2'.repeat(64)}`,
  chain: 1,
  epoch: 1,
};
const saved = {
  ...initialFirstRun('own', 'protect'),
  profile,
  serverAddress: 'localhost',
  account: { alias: 'personal', username: 'alice', deviceName: 'Device' },
};
test('entry and navigation cannot invent account prerequisites', () => {
  for (const state of [
    'account',
    'protect',
    'phrase',
    'waiting',
    'local-done',
  ] as const) {
    assert.equal(
      resolveSetupEntry(FIXTURE, null, { state, path: 'own' }).state,
      'who',
    );
    assert.equal(
      transitionFirstRun(initialFirstRun(), { type: 'navigate', state }).state,
      'who',
    );
  }
  assert.equal(
    resolveSetupEntry(
      { ...FIXTURE, profileInventoryStatus: 'unavailable' },
      saved,
      { state: 'phrase' },
    ).state,
    'protect',
  );
});
test('unrelated unavailable profiles cannot prevent account reconciliation', () => {
  const snapshot = {
    ...FIXTURE,
    profileInventoryStatus: 'complete' as const,
    catalogProfiles: ['personal', 'unrelated'],
    profileInventory: [
      {
        profile: 'personal',
        accounts: 'complete' as const,
        teams: 'complete' as const,
      },
      {
        profile: 'unrelated',
        accounts: 'unavailable' as const,
        teams: 'unavailable' as const,
      },
    ],
    servers: FIXTURE.servers.map((s) =>
      s.id === 'personal' ? { ...s, host_id: profile.hostId } : s,
    ),
    accounts: [],
  };
  assert.equal(authoritativeSetupFacts(snapshot, saved).account, 'missing');
  assert.equal(
    resolveSetupEntry(snapshot, saved, { state: 'protect' }).state,
    'account',
  );
});
