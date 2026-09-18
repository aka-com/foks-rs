import assert from 'node:assert/strict';
import test from 'node:test';
import { FIXTURE } from '../src/fixture';
import { decodeProfileReconciliation } from '../src/bridge';
import { serverFactAvailability } from '../src/model';
import {
  canonicalProbeEndpoint,
  connectionObservationFresh,
  connectionSecurityFailure,
  observeProfileConnection,
  retainProfileConnection,
} from '../src/profile-connectivity';

const SERVER = { ...FIXTURE.servers[0], host_id: `02${'11'.repeat(32)}` };
const CONNECTED = {
  status: 'connected' as const,
  hostId: SERVER.host_id,
  configuredProbe: `${SERVER.configuredProbe}:4430`,
};

const failure = (code: string) => ({
  code,
  message: 'Unavailable',
  retryable: true,
  ambiguous: false,
  fatal: false,
  details: { profile: SERVER.id },
});

test('identity reachability and compatibility renewal remain independent observations', () => {
  const server = SERVER;
  const report = decodeProfileReconciliation(
    {
      profile: server.id,
      identity: CONNECTED,
      compatibility: { status: 'failed', error: failure('capability-denied') },
    },
    server.id,
  );
  const observation = observeProfileConnection(server, report, 100);
  assert.equal(observation.status, 'observed');
  assert.equal(connectionSecurityFailure(observation), undefined);
  assert.ok(connectionObservationFresh(observation, 101));
  assert.equal(connectionObservationFresh(observation, 99), false);
  assert.equal(connectionObservationFresh(observation, 131), false);
  assert.deepEqual(server.compatibility, SERVER.compatibility);
});

test('reachability observations never grant permission or turn endpoint outages into failed trust', () => {
  const original = SERVER;
  const connectivity = observeProfileConnection(
    original,
    {
      profile: original.id,
      identity: CONNECTED,
      compatibility: { status: 'renewed' },
    },
    100,
  );
  const base = {
    ...original,
    connectivity,
    trust: { status: 'verified' as const },
    passiveStatus: {
      status: 'available' as const,
      source: 'signed-server-status' as const,
    },
  };
  assert.deepEqual(
    serverFactAvailability(
      {
        ...base,
        compatibility: {
          status: 'incompatible',
          expiresAt: 200,
          reason: 'drift',
        },
      },
      [],
      { nowSeconds: 100 },
    ),
    { available: false, reason: 'compatibility-incompatible' },
  );
  assert.deepEqual(
    serverFactAvailability(
      {
        ...base,
        compatibility: {
          status: 'required',
          expiresAt: 99,
          capabilities: ['kv'],
        },
      },
      [],
      { nowSeconds: 100 },
    ),
    { available: false, reason: 'check-in-expired' },
  );
  const network = observeProfileConnection(
    original,
    {
      profile: original.id,
      identity: { status: 'failed', error: failure('server-unavailable') },
      compatibility: { status: 'not-required' },
    },
    100,
  );
  assert.equal(
    serverFactAvailability(
      {
        ...base,
        connectivity: network,
        compatibility: { status: 'not-required' },
      },
      [],
      { nowSeconds: 100 },
    ).available,
    true,
  );
  const missing = observeProfileConnection(
    original,
    {
      profile: original.id,
      identity: { status: 'failed', error: failure('saved-trust-missing') },
      compatibility: { status: 'not-required' },
    },
    100,
  );
  assert.deepEqual(
    serverFactAvailability({ ...base, connectivity: missing }, [], {
      nowSeconds: 100,
    }),
    { available: false, reason: 'security-state-missing' },
  );
});

test('malformed and incorrectly bound connectivity results fail closed', () => {
  const good = {
    profile: 'p',
    identity: CONNECTED,
    compatibility: { status: 'not-required' },
  };
  for (const value of [
    null,
    {},
    { ...good, profile: 'q' },
    { ...good, identity: {} },
    { ...good, compatibility: { status: 'validated' } },
    { ...good, identity: { status: 'failed' } },
    { ...good, identity: { status: 'connected', error: failure('io') } },
  ])
    assert.throws(() => decodeProfileReconciliation(value, 'p'));
});

test('only security failures restrict identity; transport failure is not a trust replacement', () => {
  const server = SERVER;
  for (const code of [
    'operation-failed',
    'saved-trust-missing',
    'rollback-detected',
  ]) {
    const report = decodeProfileReconciliation(
      {
        profile: server.id,
        identity: { status: 'failed', error: failure(code) },
        compatibility: { status: 'not-required' },
      },
      server.id,
    );
    const observation = observeProfileConnection(server, report, 100);
    assert.equal(
      Boolean(connectionSecurityFailure(observation)),
      code !== 'operation-failed',
    );
  }
});

test('verified identity results bind canonical endpoints and cannot cross pin changes', () => {
  assert.equal(canonicalProbeEndpoint(' FOKS.APP. '), 'foks.app:4430');
  assert.equal(canonicalProbeEndpoint('[0:0:0:0:0:0:0:1]:9443'), '[::1]:9443');
  assert.equal(canonicalProbeEndpoint('::1'), '[::1]:4430');
  assert.equal(canonicalProbeEndpoint('https://foks.app'), null);
  assert.equal(canonicalProbeEndpoint('foks.app:0'), null);
  const report = {
    profile: SERVER.id,
    identity: CONNECTED,
    compatibility: { status: 'not-required' as const },
  };
  for (const changed of [
    { ...SERVER, host_id: 'different' },
    { ...SERVER, configuredProbe: 'other.example' },
  ])
    assert.throws(() => observeProfileConnection(changed, report, 10), {
      code: 'catalog-read-retired',
    });
  assert.throws(() =>
    decodeProfileReconciliation(
      { ...report, identity: { status: 'connected' } },
      SERVER.id,
    ),
  );
});

test('old reachability is discarded after profile address or pinned identity changes', () => {
  const server = SERVER;
  const observation = observeProfileConnection(
    server,
    {
      profile: server.id,
      identity: CONNECTED,
      compatibility: { status: 'not-required' },
    },
    10,
  );
  assert.equal(retainProfileConnection(server, observation), observation);
  for (const changed of [
    { ...server, id: 'other' },
    { ...server, configuredProbe: 'other.example' },
    { ...server, host_id: 'different-host' },
  ])
    assert.deepEqual(retainProfileConnection(changed, observation), {
      status: 'unknown',
    });
});
