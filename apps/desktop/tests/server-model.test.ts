/**
 * The pure server model behind Settings › Servers: the state a server's mark
 * and chip show, the identity the catalog pins, and the expiry and version
 * text. Loaded in node with no DOM.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { FIXTURE } from '../src/mock/fixture';
import type { AgentSnapshot, Server, ServerFailure } from '../src/model';
import {
  canReadServer,
  expires,
  isLocked,
  leaseExpiry,
  markTone,
  pinnedHost,
  resolveServerUiState,
  serverFor,
  STATE_LABEL,
  versionMismatchText,
} from '../src/features/settings/servers/server-model';

const failure: ServerFailure = {
  code: 'verification-failed',
  message: 'The server identity changed.',
  retryable: false,
  ambiguous: false,
  fatal: false,
};

function acme(): Server {
  const server = serverFor(FIXTURE, 'acme');
  assert.ok(server);
  return server;
}

function withServer(server: Server): AgentSnapshot {
  return {
    ...FIXTURE,
    servers: FIXTURE.servers.map((entry) =>
      entry.profileName === server.profileName ? server : entry,
    ),
  };
}

test('serverFor finds a server by profile and nothing without one', () => {
  assert.equal(serverFor(FIXTURE, 'acme')?.displayLabel, 'Acme');
  assert.equal(serverFor(FIXTURE, undefined), undefined);
  assert.equal(serverFor(FIXTURE, 'missing'), undefined);
});

test('a blocked or schema-incompatible server states no readable identity', () => {
  const server = acme();
  assert.equal(canReadServer(server), true);
  assert.deepEqual(pinnedHost(server), {
    hostId: 'b04d17e390',
    chain: 33,
    epoch: 90417,
  });
  const blocked: Server = {
    ...server,
    trust: { status: 'blocked', error: failure },
  };
  assert.equal(canReadServer(blocked), false);
  assert.equal(pinnedHost(blocked), null);
  const schema: Server = {
    ...server,
    restrictions: [{ kind: 'schema-incompatible', error: failure }],
  };
  assert.equal(canReadServer(schema), false);
  assert.equal(pinnedHost(schema), null);
});

test('the UI state follows the server availability', () => {
  const server = acme();
  assert.equal(resolveServerUiState(withServer(server), server), 'checked');
  const blocked: Server = {
    ...server,
    trust: { status: 'blocked', error: failure },
  };
  assert.equal(resolveServerUiState(withServer(blocked), blocked), 'blocked');
  const schema: Server = {
    ...server,
    restrictions: [{ kind: 'schema-incompatible', error: failure }],
  };
  assert.equal(resolveServerUiState(withServer(schema), schema), 'schema');
  const booting: AgentSnapshot = {
    ...FIXTURE,
    agent: { state: 'bootstrap', step: 'starting' },
  };
  assert.equal(resolveServerUiState(booting, server), 'unavailable');
});

test('only checked and pending servers are left unlocked', () => {
  for (const state of Object.keys(STATE_LABEL) as (keyof typeof STATE_LABEL)[])
    assert.equal(
      isLocked(state),
      !['checked', 'unprobed', 'pending'].includes(state),
      state,
    );
  assert.equal(markTone('checked'), 'ok');
  assert.equal(markTone('blocked'), 'bad');
  assert.equal(markTone('pending'), '');
});

test('a page check supersedes the catalog lease expiry', () => {
  const server = acme();
  assert.ok(server.compatibility.status === 'required');
  assert.equal(leaseExpiry(server), server.compatibility.expiresAt);
  assert.equal(
    leaseExpiry({ ...server, compatibility: { status: 'not-required' } }),
    null,
  );
  assert.equal(
    leaseExpiry(server, {
      profile: 'acme',
      configuredEndpoint: server.configuredEndpoint,
      host: null,
      leaseRequired: true,
      leaseExpiresAt: 1_900_000_000,
      compatibility: {
        status: 'required',
        expiresAt: 1_900_000_000,
        capabilities: [],
      },
      chatSupported: null,
    }),
    1_900_000_000,
  );
  assert.equal(expires(null), 'No expiration date');
  assert.equal(expires(Number.NaN), 'Invalid date');
});

test('a refused client version names the accepted range', () => {
  assert.equal(
    versionMismatchText({
      minimum: '0.2.0',
      newest: '0.3.1',
      message: 'Upgrade the client.',
      compatible: false,
    }),
    'This server does not accept FOKS client 0.1.9; it requires 0.2.0 or newer and supports up to 0.3.1. Upgrade the client.',
  );
  assert.equal(
    versionMismatchText({
      minimum: null,
      newest: null,
      message: '',
      compatible: false,
    }),
    'This server does not accept FOKS client 0.1.9.',
  );
});
