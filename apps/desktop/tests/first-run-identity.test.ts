import assert from 'node:assert/strict';
import test from 'node:test';
import { FIXTURE } from '../src/fixture';
import {
  resolveProvisionedIdentity,
  provisionedIdentityProblem,
} from '../src/first-run-identity';
import { identityRetryDelay } from '../src/screens/first-run/use-account-identity';
import {
  initialFirstRun,
  transitionFirstRun,
  encodeFirstRunCheckpoint,
  decodeFirstRunCheckpoint,
  reconcileFirstRunCheckpoint,
} from '../src/first-run-state';

const profile = {
  profile: 'personal',
  hostId: `02${'2'.repeat(64)}`,
  lookupName: 'localhost',
  canonicalName: 'localhost',
  acceptance: 'inserted' as const,
  chain: 1,
  epoch: 1,
};
const pending = transitionFirstRun(
  {
    ...initialFirstRun('own', 'account'),
    profile,
    serverAddress: 'localhost:4430',
  },
  { type: 'account-provisioned', alias: 'personal', deviceName: 'Mac' },
);
const snapshot = {
  ...FIXTURE,
  servers: FIXTURE.servers.map((server) =>
    server.id === 'personal' ? { ...server, host_id: profile.hostId } : server,
  ),
};

test('identity conflicts distinguish incomplete inventory from missing and inconsistent records', () => {
  assert.equal(
    provisionedIdentityProblem(
      {
        ...snapshot,
        servers: snapshot.servers.map((server) => ({
          ...server,
          host_id: null,
        })),
      },
      pending,
    ),
    'server-unverified',
  );
  assert.equal(
    provisionedIdentityProblem(
      { ...snapshot, profileInventoryStatus: 'unavailable', servers: [] },
      pending,
    ),
    'inventory-unavailable',
  );
  assert.equal(
    provisionedIdentityProblem({ ...snapshot, servers: [] }, pending),
    'profile-missing',
  );
  assert.equal(
    provisionedIdentityProblem({ ...snapshot, accounts: [] }, pending),
    'account-missing',
  );
  assert.equal(
    provisionedIdentityProblem(
      { ...snapshot, accounts: [...snapshot.accounts, snapshot.accounts[0]] },
      pending,
    ),
    'duplicate-records',
  );
  assert.equal(
    provisionedIdentityProblem(
      {
        ...snapshot,
        servers: snapshot.servers.map((s) => ({ ...s, host_id: 'wrong' })),
      },
      pending,
    ),
    'host-mismatch',
  );
});

test('acknowledged provisioning round-trips a secret-free pending checkpoint', () => {
  assert.equal(pending.state, 'identity-pending');
  assert.deepEqual(
    decodeFirstRunCheckpoint(encodeFirstRunCheckpoint(pending)),
    {
      ...pending,
      account: undefined,
      group: undefined,
    },
  );
  const withSecret = {
    ...pending,
    recoveryPhrase: 'secret',
    passphrase: 'secret',
  };
  assert.equal(encodeFirstRunCheckpoint(withSecret).includes('secret'), false);
  for (const invalid of [
    { ...pending, state: 'account' },
    { ...pending, provisionedAccount: undefined },
    { ...pending, profile: undefined },
    { ...pending, provisionedAccount: { alias: '../bad', deviceName: 'Mac' } },
    {
      ...pending,
      provisionedAccount: {
        alias: 'personal',
        deviceName: 'Mac',
        phrase: 'secret',
      },
    },
    {
      ...pending,
      account: { alias: 'personal', username: 'satoshi', deviceName: 'Mac' },
    },
  ])
    assert.equal(decodeFirstRunCheckpoint(JSON.stringify(invalid)), null);
});

test('acknowledged provisioning cannot be rewound by navigation or missing inventory', () => {
  assert.equal(
    transitionFirstRun(pending, { type: 'navigate', state: 'account' }),
    pending,
  );
  assert.equal(
    transitionFirstRun(pending, { type: 'choose', path: 'own' }),
    pending,
  );
  assert.equal(
    transitionFirstRun(pending, {
      type: 'account-complete',
      alias: 'other',
      username: 'satoshi',
      deviceName: 'Mac',
    }),
    pending,
  );
  for (const fact of ['missing', 'unknown'] as const) {
    assert.equal(
      reconcileFirstRunCheckpoint(pending, {
        profile: fact,
        account: fact,
        group: fact,
      }),
      pending,
    );
  }
});

test('identity adoption requires the pinned host, unique alias match and scoped complete inventory', () => {
  const resolved = resolveProvisionedIdentity(snapshot, pending);
  assert.equal(resolved.state, 'protect');
  assert.equal(resolved.account?.username, 'satoshi');
  assert.equal(resolved.provisionedAccount, undefined);
  for (const changed of [
    { ...snapshot, servers: [] },
    { ...snapshot, servers: [...snapshot.servers, snapshot.servers[0]] },
    {
      ...snapshot,
      servers: snapshot.servers.map((server) => ({
        ...server,
        host_id: 'different',
      })),
    },
    { ...snapshot, accounts: [] },
    { ...snapshot, accounts: [...snapshot.accounts, snapshot.accounts[0]] },
    {
      ...snapshot,
      accounts: snapshot.accounts.map((account) => ({
        ...account,
        server: 'other',
      })),
    },
    { ...snapshot, profileInventory: [] },
    {
      ...snapshot,
      profileInventory: [
        {
          profile: 'personal',
          accounts: 'unavailable' as const,
          teams: 'complete' as const,
        },
      ],
    },
  ])
    assert.equal(resolveProvisionedIdentity(changed, pending), pending);
  assert.equal(
    resolveProvisionedIdentity(
      {
        ...snapshot,
        profileInventory: snapshot.profileInventory.map((row) =>
          row.profile === 'personal'
            ? row
            : { ...row, accounts: 'unavailable' },
        ),
      },
      pending,
    ).state,
    'protect',
    'unrelated profile failure does not block identity adoption',
  );
});

test('the identity probe keeps its patience and spends it in fewer reads', () => {
  const delays: number[] = [];
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const delay = identityRetryDelay(attempt);
    if (delay === null) break;
    delays.push(delay);
  }
  // Every retry is one forced whole-catalog read, so the retry count is the
  // read count. The flat two-second schedule spent its budget in thirty.
  assert.ok(
    delays.length < 12,
    `${delays.length} retries still reads the catalog too often`,
  );
  const patience = delays.reduce((total, delay) => total + delay, 0);
  assert.ok(
    patience >= 60_000,
    `${patience}ms is less patient than the flat schedule was`,
  );
  // The first wait is unchanged, because these failures almost always clear
  // on the first retry and that case must not get slower.
  assert.equal(delays[0], 2_000);
  // Geometric until the ceiling, then flat: without the ceiling the last
  // waits would dominate the budget and a late-clearing failure would be
  // reported long after it cleared.
  assert.deepEqual(delays.slice(0, 3), [2_000, 4_000, 8_000]);
  assert.ok(delays.every((delay) => delay <= 8_000));
  assert.ok(
    delays.slice(3).every((delay) => delay === 8_000),
    'the ceiling is not holding',
  );
  // The budget is a bound, not a suggestion.
  assert.equal(identityRetryDelay(delays.length), null);
});
