import assert from 'node:assert/strict';
import test from 'node:test';
import { FIXTURE } from '../src/fixture';
import { resolveProvisionedIdentity } from '../src/first-run-identity';
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
const world = {
  ...FIXTURE,
  servers: FIXTURE.servers.map((server) =>
    server.id === 'personal' ? { ...server, host_id: profile.hostId } : server,
  ),
};

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
      account: { alias: 'personal', username: 'rae', deviceName: 'Mac' },
    },
  ])
    assert.equal(decodeFirstRunCheckpoint(JSON.stringify(invalid)), null);
});

test('acknowledged provisioning cannot be rewound by navigation or missing inventory', () => {
  assert.equal(
    transitionFirstRun(pending, { type: 'go', state: 'account' }),
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
      username: 'rae',
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
  const resolved = resolveProvisionedIdentity(world, pending);
  assert.equal(resolved.state, 'protect');
  assert.equal(resolved.account?.username, 'rae');
  assert.equal(resolved.provisionedAccount, undefined);
  for (const changed of [
    { ...world, servers: [] },
    { ...world, servers: [...world.servers, world.servers[0]] },
    {
      ...world,
      servers: world.servers.map((server) => ({
        ...server,
        host_id: 'different',
      })),
    },
    { ...world, accounts: [] },
    { ...world, accounts: [...world.accounts, world.accounts[0]] },
    {
      ...world,
      accounts: world.accounts.map((account) => ({
        ...account,
        server: 'other',
      })),
    },
    { ...world, profileInventory: [] },
    {
      ...world,
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
        ...world,
        profileInventory: world.profileInventory.map((row) =>
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
