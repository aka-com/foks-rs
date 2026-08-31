import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';

import {
  decodeAccounts,
  decodeCatalog,
  decodeCopy,
  decodeDownload,
  decodeMutation,
  decodeCheckedProfile,
  decodePendingOperations,
  decodeBackupPhrase,
  decodeGroupDiscovery,
  decodeServerStatus,
  decodeCheckedServer,
  decodeAccountDevices,
  decodeBackupEnrollments,
  decodePairingOffer,
  decodeDeviceProvision,
  decodeDeviceRemoval,
  decodePassphraseReport,
  decodeResetPreview,
  decodeYubiCards,
  decodeYubiAccounts,
  decodeYubiResult,
  decodeAppInfo,
  decodeAppLockState,
  decodeReadItem,
  decodeServers,
  decodeParties,
  decodeFederation,
  loadWorld,
  normalizeCommandError,
  roleDto,
  selectBridge,
  tauriBridge,
} from '../src/bridge';
import type { Bridge, CatalogDto } from '../src/bridge';
import { signedLeaseState } from '../src/model';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';

const catalog: CatalogDto = {
  profiles: ['foks.example.net'],
  stores: [{
    id: '{"Account":{"profile":"foks.example.net","account_alias":"rae"}}',
    kind: 'account',
    name: 'Personal',
    server: 'foks.example.net',
    account: 'rae',
  }],
  items: [{
    store: '{"Account":{"profile":"foks.example.net","account_alias":"rae"}}',
    path: '/logins/example.test',
    kind: 'Secret',
    size: 42,
    version: 3,
    read: { role: 'Owner' },
    write: { role: 'Owner' },
  }],
  failures: [],
  blockedProfiles: [],
};

const checkedHost = {
  lookupName: 'foks.example.net',
  canonicalName: 'foks.example.net',
  hostId: `02${'1'.repeat(64)}`,
  chain: 1,
  epoch: 1,
};

test('the Rust catalog wire shape decodes and preserves opaque store ids', () => {
  const decoded = decodeCatalog(catalog);
  assert.deepEqual(decoded, catalog);
  assert.equal(decoded.items[0]?.store, decoded.stores[0]?.id);
});

test('catalog drift fails closed at the bridge', () => {
  assert.throws(
    () => decodeCatalog({ ...catalog, blocked_profiles: [], blockedProfiles: undefined }),
    /blockedProfiles must be an array/,
  );
  assert.equal(
    decodeCatalog({
      ...catalog,
      items: [{ ...catalog.items[0], read: { role: 'Member', visibility: -16384 } }],
    }).items[0]?.read.role,
    'Member',
  );
  assert.throws(() => decodeCatalog({
    ...catalog,
    items: [{ ...catalog.items[0], read: { role: 'Member' } }],
  }), /visibility must be a signed 16-bit integer/);
  assert.throws(() => decodeCatalog({
    ...catalog,
    items: [{ ...catalog.items[0], read: { role: 'Member', visibility: 32768 } }],
  }), /visibility must be a signed 16-bit integer/);
  assert.throws(
    () => decodeCatalog({ ...catalog, items: [{ ...catalog.items[0], version: 3.5 }] }),
    /version must be a non-negative safe integer/,
  );
});

test('action response decoders reject optimistic or partial shapes', () => {
  assert.deepEqual(decodeReadItem({
    store: catalog.stores[0]?.id,
    path: '/logins/example.test',
    version: 3,
    value: 'secret',
  }), {
    store: catalog.stores[0]?.id,
    path: '/logins/example.test',
    version: 3,
    value: 'secret',
  });
  assert.throws(() => decodeReadItem({ value: 'secret' }), /store must be a string/);
  assert.deepEqual(decodeCopy({ ok: true }), { ok: true });
  assert.throws(() => decodeCopy({ ok: false }), /must be true/);
  assert.deepEqual(decodeDownload({ saved: false }), { saved: false });
  assert.deepEqual(decodeMutation({ applied: true }), { applied: true });
  assert.deepEqual(decodeMutation({ applied: false }), { applied: false });
  assert.throws(() => decodeMutation({ ok: true }), /applied must be a boolean/);
});

test('app-lock responses are coherent and fail closed on wire drift', () => {
  assert.deepEqual(decodeAppLockState({
    locked: true,
    available: true,
    mechanism: 'biometry',
  }), {
    locked: true,
    available: true,
    mechanism: 'biometry',
  });
  assert.deepEqual(decodeAppLockState({
    locked: false,
    available: false,
    mechanism: 'none',
    unavailableReason: 'No authenticator is installed.',
  }), {
    locked: false,
    available: false,
    mechanism: 'none',
    unavailableReason: 'No authenticator is installed.',
  });
  assert.throws(
    () => decodeAppLockState({ locked: true, available: false, mechanism: 'none' }),
    /unavailable app lock cannot be armed/,
  );
  assert.throws(
    () => decodeAppLockState({ locked: false, available: true, mechanism: 'none' }),
    /capability and mechanism disagree/,
  );
});

test('server, roster and federation wire shapes validate signed roles', () => {
  assert.deepEqual(decodeAccounts([{
    store: 'opaque-account-ref', profile: 'foks.example', alias: 'personal', username: 'rae.chen',
  }]), [{ store: 'opaque-account-ref', server: 'foks.example', alias: 'personal', username: 'rae.chen' }]);
  assert.throws(() => decodeAccounts([{ profile: 'foks.example', alias: 'personal', username: 'rae.chen' }]), /store must be a string/);
  const servers = decodeServers([{
    id: 'foks.example.net', name: 'foks.example.net', label: null,
    host_id: 'abc', chain: 2, epoch: 9, lease: null, accounts: ['rae'], state: 'ok',
  }]);
  assert.equal(servers[0]?.host_id, 'abc');
  const parties = decodeParties([{
    store: catalog.stores[0]?.id,
    username: 'deploy-bot',
    party_kind: 'user',
    generation: 4,
    locally_manageable: true,
    party_id_hex: '01aa',
    source_role: { role: 'Member', visibility: -16384 },
    destination_role: { role: 'Member', visibility: -16384 },
  }]);
  assert.equal(parties[0]?.username, 'deploy-bot');
  assert.deepEqual(parties[0]?.source_role, { role: 'Member', visibility: -16384 });
  const federation = decodeFederation([{
    store: catalog.stores[0]?.id,
    remote_profile: 'elsewhere', remote_team_alias: 'ops',
    remote_host_id_hex: 'aa', remote_team_id_hex: 'bb',
    destination: { role: 'Admin' }, active: false,
  }]);
  assert.equal(federation[0]?.operation_id_hex, undefined);
  assert.throws(() => decodeServers([{ ...servers[0], state: 'mystery' }]), /server state/);
});

test('the TypeScript decoders accept the Rust-serialized wire golden', async () => {
  const fixture = JSON.parse(
    await readFile(new URL('../../foks-tauri/wire-contract.json', import.meta.url), 'utf8'),
  ) as Record<string, unknown>;
  const role = decodeCatalog(fixture.catalog).items[0]?.read;
  assert.ok(role && role.role === 'Member');
  assert.equal(role.visibility, -16384);
  assert.deepEqual(decodeAccounts([fixture.account]), [{
    store: 'opaque-account-ref', server: 'foks.example', alias: 'personal', username: 'rae.chen',
  }]);
  assert.equal(decodeReadItem(fixture.readItem).version, 7);
  assert.deepEqual(decodeCopy(fixture.commandAck), { ok: true });
  assert.deepEqual(decodeDownload(fixture.cancelledDownload), { saved: false });
  assert.deepEqual(decodeMutation(fixture.appliedMutation), { applied: true });
  assert.deepEqual(decodeMutation(fixture.cancelledMutation), { applied: false });
  assert.equal(normalizeCommandError(fixture.alreadyExistsError).code, 'already-exists');
  assert.equal(normalizeCommandError(fixture.agentLostError).code, 'agent-lost');
  assert.equal(normalizeCommandError(fixture.commandError).code, 'version-mismatch');
  assert.equal(decodeCheckedProfile(fixture.checkedProfile).acceptance, 'inserted');
  assert.equal(decodePendingOperations([fixture.pendingOperation])[0]?.kind, 'account-recovery');
  assert.equal(decodeBackupPhrase(fixture.backupPhrase).backupAlias, 'paper');
  assert.equal(decodeGroupDiscovery(fixture.groupDiscovery).groups[0]?.name, 'Engineering');
  assert.deepEqual(decodeAppInfo(fixture.appInfo), {
    version: '0.3.0', agentSocket: '/private/foks/agent.sock', managedProfile: 'local',
  });
  assert.equal(decodeServerStatus(fixture.serverStatus).profile, 'work');
  assert.equal(decodeCheckedServer(fixture.checkedServer).acceptance, 'advanced');
  assert.equal(decodeAccountDevices([fixture.device])[0]?.name, 'This Mac');
  assert.equal(decodeBackupEnrollments([fixture.backupEnrollment])[0]?.backupAlias, 'paper');
  assert.equal(decodePairingOffer(fixture.pairingOffer).accountAlias, 'personal');
  assert.match(decodePairingOffer(fixture.pairingOffer).phrase, /\S/);
  assert.equal(decodeDeviceProvision(fixture.deviceProvision).userChainSequence, 14);
  assert.equal(decodeDeviceRemoval(fixture.deviceRemoval).userChainSequence, 15);
  assert.equal(decodePassphraseReport(fixture.passphraseReport).verified, true);
  assert.equal(decodeResetPreview(fixture.resetPreview).token.length > 0, true);
  assert.equal(decodeYubiCards([fixture.yubiCard]).length, 1);
  assert.equal(decodeYubiAccounts([fixture.yubiEnrollment])[0]?.state, 'complete');
  assert.equal(decodeYubiResult('create_yubi_account', fixture.yubiAccount).username, 'rae');
  assert.equal(decodeYubiResult('sync_yubi_account', fixture.yubiSync).entries, 7);
  assert.equal(decodeYubiResult('yubi_pin_status', fixture.yubiPinStatus).remaining, 3);
  assert.equal(decodeYubiResult('change_yubi_pin', fixture.yubiPinStatus).blocked, false);
  assert.equal(decodeYubiResult('unblock_yubi_pin', fixture.yubiPinStatus).remaining, 3);
  assert.equal(decodeYubiResult('set_yubi_passphrase', fixture.passphraseReport).verified, true);
  assert.equal(decodeYubiResult('change_yubi_passphrase', fixture.passphraseReport).generation, 2);
  assert.equal(decodeYubiResult('verify_yubi_passphrase', fixture.passphraseReport).verified, true);
  assert.equal(decodeYubiResult('change_yubi_puk', fixture.yubiChanged).changed, true);
  assert.equal(decodeYubiResult('rotate_yubi_management_key', fixture.yubiLifecycle).managementGeneration, 4);
  assert.equal(decodeYubiResult('resume_yubi_management_key', fixture.yubiLifecycle).managementEnrolled, true);
  assert.equal(decodeYubiResult('recover_yubi_management_key', fixture.yubiLifecycle).alias, 'work_key');
  assert.equal(decodeYubiResult('recover_yubi_subkey', fixture.yubiSubkeyRecovery).certificateCount, 2);
  assert.equal(decodeYubiResult('revoke_yubi_device', fixture.yubiRevocation).removedLocalCredential, true);
});

test('Phase 6 decoders reject invented status, replayable reset, and loose identities', () => {
  assert.throws(() => decodeServerStatus({ profile: 'p', configuredProbe: 'x', host: { lookupName: 'x', canonicalName: 'x', hostId: 'short', chain: 1, epoch: 2 }, leaseRequired: false, leaseExpiresAt: null }), /canonical 02 entity id/);
  assert.throws(() => decodeServerStatus({ profile: 'p', configuredProbe: 'x', host: null, leaseExpiresAt: null }), /leaseRequired must be a boolean/);
  assert.throws(() => decodeServerStatus({ profile: 'p', configuredProbe: 'x', host: null, leaseRequired: false, leaseExpiresAt: 100 }), /lease-free protocol/);
  assert.throws(() => decodeCheckedServer({ profile: 'p', acceptance: 'same', lookupName: 'x', canonicalName: 'x', hostId: `02${'1'.repeat(64)}`, chain: 1, epoch: 2 }), /inserted, advanced, or unchanged/);
  assert.throws(() => decodeResetPreview({ profile: 'p', resumables: [], artifacts: [], token: 7, expiresInSeconds: 60 }), /token must be a string/);
  assert.throws(() => decodeAccountDevices([{ id: 'd', role: 'superuser', current: true }]), /role is invalid/);
  assert.equal(decodeAccountDevices([{ id: `08${'8'.repeat(66)}`, role: 'owner', current: false }])[0]?.id.startsWith('08'), true);
  assert.throws(() => decodeAccountDevices([{ id: `08${'8'.repeat(64)}`, role: 'owner', current: false }]), /software-device or YubiKey id/);
  assert.throws(() => decodeDeviceRemoval({ deviceId: `08${'8'.repeat(66)}`, userChainSequence: 1, alreadyAbsent: false }), /canonical 04 entity id/);
  assert.throws(() => decodeYubiAccounts([{ alias: 'key', state: 'unknown' }]), /state is invalid/);
  assert.throws(() => decodeYubiResult('create_yubi_account', {
    alias: 'key', username: 'rae', yubiId: `08${'8'.repeat(64)}`,
    subkeyId: `0d${'d'.repeat(64)}`, userChainSequence: 1,
    managementEnrolled: true,
  }), /canonical YubiKey id/);
});

test('first-run response decoders reject drift and mismatched discovery identities', () => {
  assert.equal(decodeCheckedProfile({
    profile: 'work', acceptance: 'advanced', lookupName: 'foks.example',
    canonicalName: 'foks.example', hostId: `02${'1'.repeat(64)}`, chain: 1,
    epoch: 1,
  }).acceptance, 'advanced');
  assert.equal(decodeCheckedProfile({
    profile: 'work', acceptance: 'unchanged', lookupName: 'foks.example',
    canonicalName: 'foks.example', hostId: `02${'1'.repeat(64)}`, chain: 1,
    epoch: 1,
  }).acceptance, 'unchanged');
  assert.throws(() => decodeCheckedProfile({
    profile: 'work', acceptance: 'invented', lookupName: 'foks.example',
    canonicalName: 'foks.example', hostId: `02${'1'.repeat(64)}`, chain: 1,
    epoch: 1,
  }), /acceptance is invalid/);
  assert.throws(() => decodeCheckedProfile({
    profile: 'work', acceptance: 'inserted', lookupName: 'foks.example',
    canonicalName: 'foks.example', hostId: `03${'1'.repeat(64)}`, chain: 1,
    epoch: 1,
  }), /canonical 02 entity id/);
  assert.throws(() => decodePendingOperations([{ kind: 'invented', alias: 'work' }]), /pending-operation kind/);
  assert.throws(() => decodeGroupDiscovery({
    accountAlias: 'personal',
    groups: [{
      alias: 'ops', accountAlias: 'other',
      teamIdHex: `03${'1'.repeat(64)}`, kind: 'named', name: 'Ops', active: true,
    }],
  }), /different account/);
  assert.throws(() => decodeGroupDiscovery({
    accountAlias: 'personal',
    groups: [{
      alias: 'ops', accountAlias: 'personal',
      teamIdHex: `14${'1'.repeat(64)}`, kind: 'adhoc', name: 'Ops', active: true,
    }],
  }), /name must be present only/);
  assert.throws(() => decodeGroupDiscovery({
    accountAlias: 'personal',
    groups: [{
      alias: 'ops', accountAlias: 'personal',
      teamIdHex: `14${'1'.repeat(64)}`, kind: 'named', name: 'Ops', active: true,
    }],
  }), /canonical 03 entity id/);
});

test('catalog failures are structural and retain typed error details', () => {
  const decoded = decodeCatalog({
    ...catalog,
    failures: [{
      scope: 'profile', profile: 'foks.example.net', source: 'KV catalog',
      error: {
        code: 'capability-denied', message: 'KV unavailable', retryable: false,
        ambiguous: false, fatal: true,
        details: { capability: 'kv', profile: 'foks.example.net' },
      },
    }],
  });
  assert.equal(decoded.failures[0]?.error.details?.capability, 'kv');
  assert.throws(() => decodeCatalog({
    ...catalog,
    failures: [{
      scope: 'profile',
      profile: 'x',
      error: {
        code: 'failure', message: 'failed', retryable: false, ambiguous: false, fatal: true,
      },
    }],
  }), /source must be a string/);
});

test('typed Rust errors survive and malformed errors become fatal', () => {
  assert.deepEqual(normalizeCommandError({
    code: 'lease-lapsed',
    message: 'Reads and writes have stopped.',
    retryable: true,
    ambiguous: false,
    fatal: false,
  }), {
    code: 'lease-lapsed',
    message: 'Reads and writes have stopped.',
    retryable: true,
    ambiguous: false,
    fatal: false,
  });
  assert.equal(normalizeCommandError('oops').fatal, true);
});

test('a Tauri development host selects native commands, never the dev mock', async () => {
  const previous = globalThis.window;
  Object.defineProperty(globalThis, 'window', {
    value: { __TAURI_INTERNALS__: {} },
    configurable: true,
  });
  try {
    assert.equal(await selectBridge(), tauriBridge);
  } finally {
    if (previous === undefined) delete (globalThis as { window?: Window }).window;
    else Object.defineProperty(globalThis, 'window', { value: previous, configurable: true });
  }
});

test('loadWorld retains validated account-store identities in the mock world', async () => {
  const world = await loadWorld(mockBridge(FIXTURE), FIXTURE);
  assert.equal(world.accounts.length, FIXTURE.accounts.length);
  for (const account of world.accounts) {
    assert.ok(account.store);
    assert.equal(world.stores.find((store) => store.id === account.store)?.kind, 'account');
  }
});

test('loadWorld makes one catalog call and does not leak fixture facts native', async () => {
  let calls = 0;
  const accountStore = catalog.stores[0];
  assert.ok(accountStore);
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    agentStatus: async () => ({ phase: 'Ready' }),
    listCatalog: async () => {
      calls += 1;
      return catalog;
    },
    listStores: async () => ({ ...catalog, items: [] }),
    listServers: async () => [{
      id: 'foks.example.net', name: 'foks.example.net', label: null,
      host_id: null, chain: null, epoch: null, lease: null,
      accounts: ['rae'], state: 'never-probed',
    }],
    describeServerStatus: async () => ({
      profile: 'foks.example.net', configuredProbe: 'foks.example.net',
      host: checkedHost, leaseRequired: true, leaseExpiresAt: 2_000_000_000,
    }),
    listAccounts: async () => [{
      store: accountStore.id, server: 'foks.example.net', alias: 'rae', username: 'rae',
    }],
    listParties: async () => [],
    listFederation: async () => [],
    readItem: async () => { throw new Error('not used'); },
    copyItemValue: async () => ({ ok: true }),
    copyItemPath: async () => ({ ok: true }),
    downloadFile: async () => ({ saved: false }),
  };
  const world = await loadWorld(bridge, FIXTURE);
  assert.equal(calls, 1);
  assert.deepEqual(world.stores, catalog.stores);
  assert.deepEqual(world.accounts, [{
    store: accountStore.id, server: 'foks.example.net', alias: 'rae', username: 'rae',
  }]);
  assert.equal(world.parties.length, 0);
  assert.equal(world.notifications.length, 0);
  assert.equal(world.plaintext['acct:personal|/logins/github.com'], undefined);
});

test('loadWorld does not read a roster through a whole-profile safety block', async () => {
  const storeId = '{"Team":{"profile":"foks.example.net","team_alias":"ops"}}';
  const blocked: CatalogDto = {
    profiles: ['foks.example.net'],
    stores: [{
      id: storeId,
      kind: 'team',
      name: 'Ops',
      server: 'foks.example.net',
      account: 'rae',
      alias: 'ops',
      active: true,
      team_kind: 'named',
      team_id_hex: 'aa',
    }],
    items: [],
    failures: [],
    blockedProfiles: ['foks.example.net'],
  };
  let rosterCalls = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    agentStatus: async () => ({ phase: 'Ready' }),
    listCatalog: async () => blocked,
    listStores: async () => ({ ...blocked, items: [] }),
    listServers: async () => [{
      id: 'foks.example.net', name: 'foks.example.net', label: null,
      host_id: null, chain: null, epoch: null, lease: null,
      accounts: [], state: 'blocked',
    }],
    listAccounts: async () => [],
    listParties: async () => { rosterCalls += 1; return []; },
    listFederation: async () => { rosterCalls += 1; return []; },
    readItem: async () => { throw new Error('not used'); },
    copyItemValue: async () => ({ ok: true }),
    copyItemPath: async () => ({ ok: true }),
    downloadFile: async () => ({ saved: false }),
  };
  const world = await loadWorld(bridge);
  assert.equal(rosterCalls, 0);
  assert.equal(world.notifications[0]?.title, 'Access to foks.example.net is blocked');
});

test('native loadWorld derives leased and lease-free access per pinned protocol', async () => {
  assert.equal(signedLeaseState(101, 100), 'fresh');
  assert.equal(signedLeaseState(100, 100), 'lapsed');
  assert.equal(signedLeaseState(null, 100), 'unavailable');
  const profiles = ['fresh', 'expired', 'missing', 'v019', 'failed'];
  const stores = profiles.map((profile) => ({
    id: `account:${profile}`, kind: 'account' as const, name: profile,
    server: profile, account: profile,
  }));
  const response: CatalogDto = {
    profiles, stores,
    items: stores.map((store) => ({
      store: store.id, path: '/value', kind: 'Secret' as const, size: 1, version: 1,
      read: { role: 'Owner' as const }, write: { role: 'Owner' as const },
    })),
    failures: [], blockedProfiles: [],
  };
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base, native: true,
    listCatalog: async () => response,
    listServers: async () => profiles.map((profile) => ({
      id: profile, name: `${profile}.example`, label: null, host_id: null,
      chain: null, epoch: null, lease: null, accounts: [profile], state: 'never-probed' as const,
    })),
    describeServerStatus: async (profile) => {
      if (profile === 'failed') throw new Error('passive status transport failed');
      return {
        profile, configuredProbe: `${profile}.example`, host: checkedHost,
        leaseRequired: profile !== 'v019',
        leaseExpiresAt: profile === 'fresh' ? 200 : profile === 'expired' ? 99 : null,
      };
    },
    listAccounts: async () => stores.filter((store) => store.server === 'fresh' || store.server === 'v019').map((store) => ({
      store: store.id, server: store.server, alias: store.account, username: store.account,
    })),
    listParties: async () => [], listFederation: async () => [],
  };
  const world = await loadWorld(bridge, undefined, 100);
  assert.deepEqual(world.items.map((item) => item.store), ['account:fresh', 'account:v019']);
  assert.deepEqual(world.accounts.map((account) => account.store), ['account:fresh', 'account:v019']);
  assert.deepEqual(world.stores.map((store) => store.id), stores.map((store) => store.id), 'local aliases remain inspectable while authenticated facts are suppressed');
  assert.equal(world.servers.find((server) => server.id === 'fresh')?.state, 'ok');
  assert.equal(world.servers.find((server) => server.id === 'expired')?.state, 'lease-lapsed');
  assert.equal(world.servers.find((server) => server.id === 'missing')?.state, 'lease-unavailable');
  assert.equal(world.servers.find((server) => server.id === 'v019')?.state, 'ok');
  assert.equal(world.servers.find((server) => server.id === 'failed')?.state, 'lease-unavailable');
  assert.equal(world.leaseState, 'lapsed');
  assert.match(world.notifications.find((note) => note.id === 'status-unavailable-failed')?.detail ?? '', /transport failed/);

  const racedWorld = await loadWorld({
    ...bridge,
    listAccounts: async () => stores.map((store) => ({
      store: store.id, server: store.server, alias: store.account, username: store.account,
    })),
  }, undefined, 100);
  assert.deepEqual(
    racedWorld.accounts.map((account) => account.store),
    ['account:fresh', 'account:v019'],
    'stale account rows are discarded without blanking leased or lease-free available profiles',
  );
});

test('loadWorld requires account identities only from available profiles', async () => {
  const stores = FIXTURE.stores.filter((store) =>
    store.id === 'acct:personal' || store.id === 'acct:work' || store.id === 'team:eng',
  );
  const response: CatalogDto = {
    profiles: ['personal', 'acme'],
    stores,
    items: [],
    failures: [],
    blockedProfiles: ['acme'],
  };
  let rosterCalls = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    listCatalog: async () => response,
    listStores: async () => ({ ...response, items: [] }),
    listServers: async () => [
      { ...FIXTURE.servers[0], lease: null, state: 'never-probed' },
      { ...FIXTURE.servers[1], lease: null, state: 'blocked' },
    ],
    listAccounts: async () => [{
      store: 'acct:personal', server: 'personal', alias: 'personal', username: 'rae',
    }, {
      store: 'acct:work', server: 'acme', alias: 'work', username: 'rae.chen',
    }],
    listParties: async () => { rosterCalls += 1; return []; },
    listFederation: async () => { rosterCalls += 1; return []; },
  };
  const world = await loadWorld(bridge);
  assert.deepEqual(world.accounts.map((account) => account.store), ['acct:personal']);
  assert.equal(rosterCalls, 0);
});

test('loadWorld suppresses lapsed rosters and enriches live self and named-group labels fail closed', async () => {
  const base = mockBridge(FIXTURE);
  let rosterCalls = 0;
  const bridge: Bridge = {
    ...base,
    native: true,
    listCatalog: async () => ({
      profiles: ['personal', 'acme'],
      stores: [...FIXTURE.stores],
      items: FIXTURE.items.map((item) => ({
        store: item.store,
        path: item.path,
        kind: item.kind,
        size: item.size,
        version: item.version,
        read: roleDto(item.read),
        write: roleDto(item.write),
      })),
      failures: [],
      blockedProfiles: [],
    }),
    listAccounts: async () => [
      { store: 'acct:personal', alias: 'personal', username: 'rae', server: 'personal' },
    ],
    listServers: async () => FIXTURE.servers.map((server) =>
      server.id === 'acme' ? { ...server, lease: null, state: 'lease-lapsed' as const } : { ...server, lease: null },
    ),
    listParties: async (storeId) => {
      rosterCalls += 1;
      return FIXTURE.parties.filter((party) => party.store === storeId).map((party) => ({
        store: party.store,
        username: party.username,
        party_kind: party.party_kind,
        generation: party.generation,
        locally_manageable: party.locally_manageable,
        party_id_hex: party.party_id_hex,
        scoped_host_id_hex: party.scoped_host_id_hex,
        source_role: party.source_role,
        destination_role: party.destination_role,
      }));
    },
    listFederation: async (storeId) => FIXTURE.federation.filter((entry) => entry.store === storeId),
  };
  const world = await loadWorld(bridge);
  assert.equal(rosterCalls, 2, 'Household and Homelab read; lapsed Engineering does not');
  assert.equal(world.parties.some((party) => party.store === 'team:eng'), false);

  const freshBridge = {
    ...bridge,
    listServers: async () => FIXTURE.servers.map((server) => ({ ...server, lease: null, state: 'never-probed' as const })),
    listAccounts: async () => [
      { store: 'acct:personal', alias: 'personal', username: 'rae', server: 'personal' },
      { store: 'acct:work', alias: 'work', username: 'rae.chen', server: 'acme' },
    ],
    describeServerStatus: async (profile: string) => ({
      profile, configuredProbe: profile,
      host: profile === 'partner' ? null : { ...checkedHost, lookupName: profile, canonicalName: profile },
      leaseRequired: true,
      leaseExpiresAt: profile === 'partner' ? null : 2_000_000_000,
    }),
    listParties: async (storeId: string) => {
      const current = await bridge.listParties(storeId);
      if (storeId !== 'team:eng') return current;
      const local = current.find((party) => party.username === 'rae.chen');
      assert.ok(local);
      return [
        ...current,
        {
          ...local,
          label: 'you',
          locally_manageable: false,
          scoped_host_id_hex: 'remote-host',
          party_id_hex: `01${'9'.repeat(64)}`,
        },
      ];
    },
  };
  const fresh = await loadWorld(freshBridge);
  const sameNames = fresh.parties.filter((party) => party.username === 'rae.chen');
  assert.equal(sameNames.find((party) => party.locally_manageable)?.label, 'you');
  assert.equal(sameNames.find((party) => !party.locally_manageable)?.label, undefined);
  assert.equal(
    fresh.parties.find((party) => party.party_kind === 'named-team')?.team_name,
    'homelab @ foks.example.net',
  );
});

test('mock create is visible in the catalog and supports an exact-version read', async () => {
  const bridge = mockBridge(FIXTURE);
  await bridge.createTextItem({
    storeId: 'acct:personal', path: '/agents/new-key', value: 'new value',
  });
  const created = (await bridge.listCatalog()).items.find((item) => item.path === '/agents/new-key');
  assert.ok(created);
  assert.equal(created.version, 1);
  assert.equal((await bridge.readItem({ storeId: created.store, path: created.path, version: created.version })).value, 'new value');
});

test('group create primitives require and retain explicit read and write roles', async () => {
  const bridge = mockBridge(FIXTURE);
  await bridge.createTextItem({
    storeId: 'team:eng', path: '/shared/text', value: 'value',
    readRole: 'Member:-32768', writeRole: 'Admin',
  });
  await bridge.createLink({
    storeId: 'team:eng', path: '/shared/link', target: '/shared/text',
    readRole: 'Admin', writeRole: 'Member:32767',
  });
  await bridge.importDroppedFile({
    storeId: 'team:eng', path: '/shared/file', sourcePath: '/tmp/file',
    readRole: 'Member:0', writeRole: 'Admin',
  });
  await bridge.createFolder({
    storeId: 'team:eng', path: '/shared/folder',
    readRole: 'Owner', writeRole: 'Owner',
  });
  const created = (await bridge.listCatalog()).items.filter((item) =>
    item.store === 'team:eng' && item.path.startsWith('/shared/'),
  );
  assert.deepEqual(
    created.map(({ path, read, write }) => ({ path, read, write })),
    [
      { path: '/shared/text', read: { role: 'Member', visibility: -32768 }, write: { role: 'Admin' } },
      { path: '/shared/link', read: { role: 'Admin' }, write: { role: 'Member', visibility: 32767 } },
      { path: '/shared/file', read: { role: 'Member', visibility: 0 }, write: { role: 'Admin' } },
    ],
  );
  assert.equal(
    created.some(({ path }) => path === '/shared/folder'),
    false,
    'the native folder primitive does not become a fifth product item',
  );
  await assert.rejects(
    bridge.createTextItem({ storeId: 'team:eng', path: '/missing-roles', value: 'x' }),
    (error: unknown) => normalizeCommandError(error).code === 'invalid-request',
  );
  await assert.rejects(
    bridge.createTextItem({
      storeId: 'team:eng', path: '/out-of-range-role', value: 'x',
      readRole: 'Member:-32769', writeRole: 'Admin',
    }),
    (error: unknown) => normalizeCommandError(error).code === 'invalid-request',
  );
  await assert.rejects(
    bridge.createTextItem({
      storeId: 'team:eng', path: '/bad-role', value: 'x',
      readRole: 'Member: 0', writeRole: 'Admin',
    }),
    (error: unknown) => normalizeCommandError(error).code === 'invalid-request',
  );
});

test('account creates omit role inputs while group edits and replacements preserve catalog roles', async () => {
  const bridge = mockBridge(FIXTURE);
  await bridge.createTextItem({ storeId: 'acct:personal', path: '/account-default', value: 'x' });
  const account = (await bridge.listCatalog()).items.find((item) => item.path === '/account-default');
  assert.deepEqual(account?.read, { role: 'Owner' });
  assert.deepEqual(account?.write, { role: 'Owner' });
  await assert.rejects(
    bridge.createTextItem({
      storeId: 'acct:personal', path: '/account-with-role', value: 'x',
      readRole: 'Owner', writeRole: 'Owner',
    }),
    (error: unknown) => normalizeCommandError(error).code === 'invalid-request',
  );

  const text = FIXTURE.items.find((item) => item.store === 'team:eng' && item.path === '/deploy/staging-token');
  const file = FIXTURE.items.find((item) => item.store === 'team:eng' && item.path === '/release/bundle.tar');
  assert.ok(text);
  assert.ok(file);
  await bridge.editTextItem({ storeId: text.store, path: text.path, version: text.version, value: 'changed' });
  await bridge.replaceDroppedFile({ storeId: file.store, path: file.path, version: file.version, sourcePath: '/tmp/replacement' });
  const after = (await bridge.listCatalog()).items;
  const edited = after.find((item) => item.store === text.store && item.path === text.path);
  const replaced = after.find((item) => item.store === file.store && item.path === file.path);
  assert.deepEqual(edited?.read, roleDto(text.read));
  assert.deepEqual(edited?.write, roleDto(text.write));
  assert.deepEqual(replaced?.read, roleDto(file.read));
  assert.deepEqual(replaced?.write, roleDto(file.write));
});

test('mock group creation publishes kind-bound authenticated entity ids', async () => {
  const bridge = mockBridge(FIXTURE);
  await bridge.createGroup({
    accountStoreId: 'acct:personal',
    teamAlias: 'new-named',
    name: 'New Named',
    kind: 'named',
  });
  await bridge.createGroup({
    accountStoreId: 'acct:personal',
    teamAlias: 'new-adhoc',
    name: 'New Ad-hoc',
    kind: 'adhoc',
  });
  const groups = (await bridge.listCatalog()).stores.filter(
    (store) =>
      store.kind === 'team' && store.alias?.startsWith('new-') === true,
  );
  const named = groups.find((store) => store.alias === 'new-named');
  const adhoc = groups.find((store) => store.alias === 'new-adhoc');
  assert.ok(named?.team_id_hex);
  assert.ok(adhoc?.team_id_hex);
  assert.equal(named.team_id_hex.slice(0, 2), '03');
  assert.equal(adhoc.team_id_hex.slice(0, 2), '14');
});

test('mock edit advances the version and exact reads return the replacement content', async () => {
  const bridge = mockBridge(FIXTURE);
  const original = FIXTURE.items.find(
    (item) => item.store === 'acct:personal' && item.path === '/logins/github.com',
  );
  assert.ok(original);
  await bridge.editTextItem({
    storeId: original.store,
    path: original.path,
    version: original.version,
    value: 'user: marcus\npassword: replacement\nurl: https://github.com',
  });
  const edited = (await bridge.listCatalog()).items.find(
    (item) => item.store === original.store && item.path === original.path,
  );
  assert.ok(edited);
  assert.equal(edited.version, original.version + 1);
  assert.equal(
    (await bridge.readItem({
      storeId: edited.store,
      path: edited.path,
      version: edited.version,
    })).value,
    'user: marcus\npassword: replacement\nurl: https://github.com',
  );
});

test('mock native-picker import survives the required catalog refresh', async () => {
  const bridge = mockBridge(FIXTURE);
  assert.deepEqual(
    await bridge.pickAndImportFile({ storeId: 'acct:personal', path: '/documents/picked.pdf' }),
    { applied: true },
  );
  const picked = (await bridge.listCatalog()).items.find((item) => item.path === '/documents/picked.pdf');
  assert.ok(picked);
  assert.equal(picked.kind, 'File');
  assert.equal(picked.version, 1);
});

test('mock Resume marks the inactive group active for the refreshed catalog', async () => {
  const bridge = mockBridge(FIXTURE);
  await bridge.resumeGroupCreation('team:homelab');
  const store = (await bridge.listCatalog()).stores.find((candidate) => candidate.id === 'team:homelab');
  assert.ok(store && store.kind === 'team');
  assert.equal(store.active, true);
});

test('the mock rejects roster and federation mutations for an active ad-hoc group', async () => {
  const world = {
    ...FIXTURE,
    stores: FIXTURE.stores.map((store) => store.id === 'team:homelab' && store.kind === 'team'
      ? { ...store, active: true as const }
      : store),
  };
  const bridge = mockBridge(world);
  await assert.rejects(
    bridge.addGroupMember({ storeId: 'team:homelab', username: 'person', destination: { role: 'Member', visibility: 0 } }),
    (error: unknown) => normalizeCommandError(error).code === 'group-management-unavailable',
  );
  await assert.rejects(
    bridge.admitGroup({ storeId: 'team:homelab', remoteStoreId: 'team:eng', visibility: 0 }),
    (error: unknown) => normalizeCommandError(error).code === 'group-management-unavailable',
  );
});
