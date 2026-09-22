import assert from 'node:assert/strict';
import test from 'node:test';
import { FIXTURE } from '../src/fixture';
import { observeProfileConnection } from '../src/profile-connectivity';
import type {
  AgentSnapshot,
  ProtocolCapability,
  ServerFailure,
} from '../src/model/types';
import {
  WORKFLOW_REQUIREMENTS,
  requireWorkflow,
  workflowAvailability,
} from '../src/model/workflow-availability';

function snapshot(capabilities: readonly ProtocolCapability[]): AgentSnapshot {
  return {
    ...FIXTURE,
    servers: ['one', 'two'].map((profileName) => ({
      ...FIXTURE.servers[0],
      profileName,
      configuredEndpoint: profileName,
      trust: { status: 'verified' },
      compatibility: { status: 'required', expiresAt: 200, capabilities },
      passiveStatus: { status: 'available', source: 'signed-server-status' },
      connectivity: { status: 'unknown' },
      restrictions: [],
    })),
    observedExpiredLeases: [],
    accounts: ['healthy', 'locked'].map((alias) => ({
      store: `acct:${alias}`,
      alias,
      username: alias,
      server: 'one',
    })),
    storeInventory: [],
  };
}
const target = { profile: 'one', account: 'healthy', nowSeconds: 100 };
const failure: ServerFailure = {
  code: 'bot-token-locked',
  message: 'Load the token',
  retryable: true,
  ambiguous: false,
  fatal: false,
};

test('every concrete workflow checks only its required capabilities', () => {
  for (const [operation, capabilities] of Object.entries(
    WORKFLOW_REQUIREMENTS,
  )) {
    const op = operation as keyof typeof WORKFLOW_REQUIREMENTS;
    const scope =
      op === 'federate' ? { ...target, remoteProfile: 'two' } : target;
    assert.equal(
      workflowAvailability(snapshot(capabilities), op, scope).available,
      true,
      operation,
    );
    for (const denied of capabilities) {
      const access = workflowAvailability(
        snapshot(capabilities.filter((entry) => entry !== denied)),
        op,
        scope,
      );
      assert.deepEqual(
        access,
        {
          available: false,
          reason: 'capability-unavailable',
          capability: denied,
        },
        operation,
      );
    }
  }
});

test('sync, device listing, recovery and SSO do not share a broad account gate', () => {
  const current = snapshot(['user-sync', 'recovery']);
  assert.equal(
    workflowAvailability(current, 'sso-login', target).available,
    true,
  );
  assert.equal(
    workflowAvailability(current, 'backup-create', target).available,
    true,
  );
  assert.deepEqual(workflowAvailability(current, 'account-sync', target), {
    available: false,
    reason: 'capability-unavailable',
    capability: 'kv',
  });
  assert.equal(
    workflowAvailability(current, 'devices-list', target).available,
    false,
  );
  assert.equal(
    workflowAvailability(current, 'backup-revoke', target).available,
    false,
  );
});

test('both federation profiles need teams and federation', () => {
  const current = snapshot(['teams', 'federation']);
  current.servers[1].compatibility = {
    status: 'required',
    expiresAt: 200,
    capabilities: ['federation'],
  };
  assert.deepEqual(
    workflowAvailability(current, 'federate', {
      ...target,
      remoteProfile: 'two',
    }),
    {
      available: false,
      reason: 'capability-unavailable',
      capability: 'teams',
    },
  );
});

test('credentials are account scoped, not a server trust failure', () => {
  const current = snapshot(['user-sync', 'device-administration']);
  current.storeInventory = [
    {
      store: 'acct:locked',
      status: 'unavailable',
      restrictions: [],
      error: failure,
    },
  ];
  assert.deepEqual(
    workflowAvailability(current, 'web-admin-open', {
      ...target,
      account: 'locked',
    }),
    {
      available: false,
      reason: 'credentials-needed',
    },
  );
  assert.equal(
    workflowAvailability(current, 'web-admin-open', target).available,
    true,
  );
  assert.equal(
    workflowAvailability(current, 'bot-load', { ...target, account: 'locked' })
      .available,
    true,
  );
  assert.deepEqual(current.servers[0].trust, { status: 'verified' });
});

test('hardware, authentication, authorization and absent facts stay distinct', () => {
  const current = snapshot(['user-sync']);
  for (const [facts, reason] of [
    [{ hardware: 'needed' }, 'hardware-needed'],
    [{ authentication: 'needed' }, 'auth-needed'],
    [{ credentials: 'needed' }, 'credentials-needed'],
    [{ permission: 'denied' }, 'permission-denied'],
    [{ hardware: 'unknown' }, 'unknown'],
  ] as const) {
    assert.deepEqual(
      workflowAvailability(current, 'web-admin-open', { ...target, ...facts }),
      {
        available: false,
        reason,
      },
    );
  }
  assert.equal(
    workflowAvailability(current, 'sso-login', {
      ...target,
      authentication: 'needed',
    }).available,
    true,
  );
  assert.equal(
    workflowAvailability(undefined, 'web-admin-open', target).available,
    false,
  );
  assert.deepEqual(current.cardsConnected, FIXTURE.cardsConnected);
});

test('local aliases and local bot history do not need remote leases or connectivity', () => {
  const current = snapshot([]);
  current.servers[0].compatibility = { status: 'required-unavailable' };
  current.servers[0].connectivity = observeProfileConnection(
    current.servers[0],
    {
      profile: 'one',
      identity: {
        status: 'failed',
        error: { ...failure, code: 'server-unavailable' },
      },
      compatibility: { status: 'not-required' },
    },
    100,
  );
  for (const operation of [
    'local-alias',
    'bot-list',
    'bot-unload',
    'yubi-list',
    'web-admin-configure',
  ] as const)
    assert.equal(
      workflowAvailability(current, operation, target).available,
      true,
    );
  assert.equal(
    workflowAvailability(current, 'bot-enroll', target).available,
    false,
  );
});

test('dispatch rechecks the current lease and capability facts', () => {
  const current = snapshot(['recovery', 'device-administration']);
  requireWorkflow(current, 'backup-revoke', target);
  assert.throws(
    () =>
      requireWorkflow(current, 'backup-revoke', { ...target, nowSeconds: 200 }),
    /expired/,
  );
  current.servers[0].compatibility = {
    status: 'required',
    expiresAt: 200,
    capabilities: ['device-administration'],
  };
  assert.throws(
    () => requireWorkflow(current, 'backup-revoke', target),
    /recovery/,
  );
});

test('conditional passphrase and signup requirements are explicit, not guessed', () => {
  const current = snapshot(['device-administration']);
  assert.equal(
    workflowAvailability(current, 'yubi-resume', target).available,
    true,
  );
  assert.deepEqual(
    workflowAvailability(current, 'yubi-resume', {
      ...target,
      signupRequired: true,
    }),
    {
      available: false,
      reason: 'capability-unavailable',
      capability: 'signup',
    },
  );
  assert.deepEqual(
    workflowAvailability(current, 'device-remove', {
      ...target,
      passphraseRequired: true,
    }),
    {
      available: false,
      reason: 'capability-unavailable',
      capability: 'passphrases',
    },
  );
});
