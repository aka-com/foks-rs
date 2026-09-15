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
  decodeBackupRevocation,
  decodePairingOffer,
  decodeDeviceProvision,
  decodeDeviceRemoval,
  decodePassphraseReport,
  decodeResetPreview,
  decodeYubiCards,
  decodeYubiAccounts,
  decodeYubiResult,
  decodeAppInfo,
  decodeGoProfileDiscovery,
  decodeAppLockState,
  decodeAgentStatus,
  decodeMaintenanceSnapshot,
  decodeReadItem,
  decodeServers,
  decodeParties,
  decodeFederation,
  decodeGroupDetails,
  discoverUnboundTeams,
  loadSnapshot,
  enqueueProfileWork,
  normalizeCommandError,
  onAgentReadinessRequired,
  shouldReportPassiveServerStatusError,
  roleDto,
  selectBridge,
  tauriBridge,
} from '../src/bridge';
import type { Bridge, CatalogDto } from '../src/bridge';
import {
  canCreateInStore,
  signedLeaseState,
  serverAvailability,
  storeReadable,
  type Server,
  type AgentSnapshot,
} from '../src/model';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';

const catalog: CatalogDto = {
  profiles: ['foks.example.net'],
  stores: [
    {
      id: '{"Account":{"profile":"foks.example.net","account_alias":"satoshi"}}',
      kind: 'account',
      name: 'Personal',
      server: 'foks.example.net',
      account: 'satoshi',
    },
  ],
  knownStores: [
    {
      id: '{"Account":{"profile":"foks.example.net","account_alias":"satoshi"}}',
      kind: 'account',
      name: 'Personal',
      server: 'foks.example.net',
      account: 'satoshi',
    },
  ],
  inventory: [
    {
      profile: 'foks.example.net',
      accountsComplete: true,
      teamsComplete: true,
    },
  ],
  items: [
    {
      store:
        '{"Account":{"profile":"foks.example.net","account_alias":"satoshi"}}',
      path: '/logins/example.test',
      kind: 'Secret',
      size: 42,
      version: 3,
      read: { role: 'Owner' },
      write: { role: 'Owner' },
    },
  ],
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

function listedServer(
  id: string,
  state:
    | 'ok'
    | 'lease-lapsed'
    | 'lease-unavailable'
    | 'never-probed'
    | 'blocked' = 'never-probed',
  accounts: string[] = [],
): Server {
  const error = {
    code: 'server-verification-failed',
    message: 'verification failed',
    retryable: false,
    ambiguous: false,
    fatal: false,
  };
  return {
    id,
    name: id,
    label: null,
    host_id: null,
    chain: null,
    epoch: null,
    accounts,
    trust:
      state === 'blocked'
        ? { status: 'blocked', error }
        : state === 'never-probed'
          ? { status: 'unprobed' }
          : { status: 'verified' },
    compatibility:
      state === 'lease-lapsed'
        ? { status: 'required', expiresAt: 0 }
        : state === 'lease-unavailable'
          ? { status: 'required-unavailable' }
          : { status: 'not-required' },
    passiveStatus: { status: 'available', source: 'signed-server-status' },
    connectivity: { status: 'unknown' },
    capabilities: { chat: false },
    restrictions: [],
  };
}

test('decodeCatalog parses catalog payload and preserves store IDs', () => {
  const decoded = decodeCatalog(catalog);
  assert.deepEqual(decoded, catalog);
  assert.equal(decoded.items[0]?.store, decoded.stores[0]?.id);
});

test('decodeCatalog preserves unavailable sizes without inventing zero', () => {
  const decoded = decodeCatalog({
    ...catalog,
    items: [{ ...catalog.items[0], size: null }],
  });
  assert.equal(decoded.items[0].size, null);
  assert.throws(
    () =>
      decodeCatalog({
        ...catalog,
        items: [{ ...catalog.items[0], size: undefined }],
      }),
    /size/,
  );
});

test('decodeCatalog enforces schema validation and rejects malformed fields', () => {
  assert.throws(
    () =>
      decodeCatalog({
        ...catalog,
        blocked_profiles: [],
        blockedProfiles: undefined,
      }),
    /blockedProfiles must be an array/,
  );
  assert.equal(
    decodeCatalog({
      ...catalog,
      items: [
        { ...catalog.items[0], read: { role: 'Member', visibility: -16384 } },
      ],
    }).items[0]?.read.role,
    'Member',
  );
  assert.throws(
    () =>
      decodeCatalog({
        ...catalog,
        items: [{ ...catalog.items[0], read: { role: 'Member' } }],
      }),
    /visibility must be a signed 16-bit integer/,
  );
  assert.throws(
    () =>
      decodeCatalog({
        ...catalog,
        items: [
          { ...catalog.items[0], read: { role: 'Member', visibility: 32768 } },
        ],
      }),
    /visibility must be a signed 16-bit integer/,
  );
  assert.throws(
    () =>
      decodeCatalog({
        ...catalog,
        items: [{ ...catalog.items[0], version: 3.5 }],
      }),
    /version must be a non-negative safe integer/,
  );
});

test('action decoders reject partial or malformed responses', () => {
  assert.deepEqual(
    decodeReadItem({
      store: catalog.stores[0]?.id,
      path: '/logins/example.test',
      version: 3,
      value: 'secret',
    }),
    {
      store: catalog.stores[0]?.id,
      path: '/logins/example.test',
      version: 3,
      value: 'secret',
    },
  );
  assert.throws(
    () => decodeReadItem({ value: 'secret' }),
    /store must be a string/,
  );
  assert.deepEqual(decodeCopy({ ok: true }), { ok: true });
  assert.throws(() => decodeCopy({ ok: false }), /must be true/);
  assert.deepEqual(decodeDownload({ saved: false }), { saved: false });
  assert.deepEqual(decodeMutation({ applied: true }), { applied: true });
  assert.deepEqual(decodeMutation({ applied: false }), { applied: false });
  assert.throws(
    () => decodeMutation({ ok: true }),
    /applied must be a boolean/,
  );
});

test('decodeGroupDetails handles independent success and error states for parties and federation', () => {
  const details = decodeGroupDetails({
    parties: { status: 'success', value: [] },
    federation: {
      status: 'error',
      error: {
        code: 'rate-limited',
        message: 'wait',
        retryable: true,
        ambiguous: false,
        fatal: false,
        details: { reason: 'status 1012' },
      },
    },
  });
  assert.deepEqual(details.parties, { status: 'success', value: [] });
  assert.equal(details.federation.status, 'error');
  if (details.federation.status === 'error') {
    assert.equal(details.federation.error.code, 'rate-limited');
    assert.equal(details.federation.error.details?.reason, 'status 1012');
  }
  assert.throws(() =>
    decodeGroupDetails({
      parties: { status: 'future', value: [] },
      federation: { status: 'success', value: [] },
    }),
  );
});

test('decodeAppLockState validates state consistency and rejects invalid combinations', () => {
  assert.deepEqual(
    decodeAppLockState({
      locked: true,
      available: true,
      mechanism: 'biometry',
    }),
    {
      locked: true,
      available: true,
      mechanism: 'biometry',
    },
  );
  assert.deepEqual(
    decodeAppLockState({
      locked: false,
      available: false,
      mechanism: 'none',
      unavailableReason: 'No authenticator is installed.',
    }),
    {
      locked: false,
      available: false,
      mechanism: 'none',
      unavailableReason: 'No authenticator is installed.',
    },
  );
  assert.throws(
    () =>
      decodeAppLockState({ locked: true, available: false, mechanism: 'none' }),
    /app_lock_state cannot be locked when lock is unavailable/,
  );
  assert.throws(
    () =>
      decodeAppLockState({ locked: false, available: true, mechanism: 'none' }),
    /capability and mechanism are incompatible/,
  );
});

test('decoders validate server status, member rosters, and federation entries', () => {
  assert.deepEqual(
    decodeAccounts([
      {
        store: 'opaque-account-ref',
        profile: 'foks.example',
        alias: 'personal',
        username: 'vitalik',
      },
    ]),
    [
      {
        store: 'opaque-account-ref',
        server: 'foks.example',
        alias: 'personal',
        username: 'vitalik',
      },
    ],
  );
  assert.throws(
    () =>
      decodeAccounts([
        { profile: 'foks.example', alias: 'personal', username: 'vitalik' },
      ]),
    /store must be a string/,
  );
  const servers = decodeServers([
    {
      id: 'foks.example.net',
      name: 'foks.example.net',
      label: null,
      host_id: 'abc',
      chain: 2,
      epoch: 9,
      lease: null,
      accounts: ['satoshi'],
      state: 'ok',
      chat_available: true,
    },
  ]);
  assert.equal(servers[0]?.host_id, 'abc');
  const parties = decodeParties([
    {
      store: catalog.stores[0]?.id,
      username: 'deploy-bot',
      party_kind: 'user',
      generation: 4,
      locally_manageable: true,
      party_id_hex: '01aa',
      source_role: { role: 'Member', visibility: -16384 },
      destination_role: { role: 'Member', visibility: -16384 },
    },
  ]);
  assert.equal(parties[0]?.username, 'deploy-bot');
  assert.deepEqual(parties[0]?.source_role, {
    role: 'Member',
    visibility: -16384,
  });
  const federation = decodeFederation([
    {
      store: catalog.stores[0]?.id,
      remote_profile: 'elsewhere',
      remote_team_alias: 'ops',
      remote_host_id_hex: 'aa',
      remote_team_id_hex: 'bb',
      destination: { role: 'Admin' },
      active: false,
    },
  ]);
  assert.equal(federation[0]?.operation_id_hex, undefined);
  assert.throws(
    () => decodeServers([{ ...servers[0], state: 'mystery' }]),
    /server state/,
  );
});

test('decoders successfully parse the full wire contract golden fixture', async () => {
  const fixture = JSON.parse(
    await readFile(
      new URL('../src-tauri/wire-contract.json', import.meta.url),
      'utf8',
    ),
  ) as Record<string, unknown>;
  const role = decodeCatalog(fixture.catalog).items[0]?.read;
  assert.ok(role && role.role === 'Member');
  assert.equal(role.visibility, -16384);
  assert.deepEqual(decodeAccounts([fixture.account]), [
    {
      store: 'opaque-account-ref',
      server: 'foks.example',
      alias: 'personal',
      username: 'vitalik',
    },
  ]);
  assert.equal(decodeReadItem(fixture.readItem).version, 7);
  assert.deepEqual(decodeCopy(fixture.commandAck), { ok: true });
  assert.deepEqual(decodeDownload(fixture.cancelledDownload), { saved: false });
  assert.deepEqual(decodeMutation(fixture.appliedMutation), { applied: true });
  assert.deepEqual(decodeMutation(fixture.cancelledMutation), {
    applied: false,
  });
  assert.equal(
    normalizeCommandError(fixture.alreadyExistsError).code,
    'already-exists',
  );
  assert.equal(
    normalizeCommandError(fixture.agentLostError).code,
    'agent-lost',
  );
  assert.equal(
    normalizeCommandError(fixture.commandError).code,
    'version-mismatch',
  );
  assert.equal(
    decodeCheckedProfile(fixture.checkedProfile).acceptance,
    'inserted',
  );
  assert.equal(
    decodePendingOperations([fixture.pendingOperation])[0]?.kind,
    'account-recovery',
  );
  assert.equal(decodeBackupPhrase(fixture.backupPhrase).backupAlias, 'paper');
  assert.equal(
    decodeGroupDiscovery(fixture.groupDiscovery).groups[0]?.name,
    'Engineering',
  );
  assert.deepEqual(decodeAppInfo(fixture.appInfo), {
    version: '0.3.0',
    agentSocket: '/private/foks/agent.sock',
    managedProfile: 'local',
  });
  assert.equal(
    decodeGoProfileDiscovery(fixture.goProfileDiscovery).candidates[0]
      ?.candidateId,
    'a'.repeat(64),
  );
  assert.equal(decodeServerStatus(fixture.serverStatus).profile, 'work');
  assert.equal(
    decodeCheckedServer(fixture.checkedServer).acceptance,
    'advanced',
  );
  assert.deepEqual(decodeCheckedServer(fixture.checkedServer).serverVersion, {
    minimum: null,
    newest: null,
    message: '',
    compatible: true,
  });
  assert.equal(decodeAccountDevices([fixture.device])[0]?.name, 'This Mac');
  assert.equal(
    decodeBackupEnrollments([fixture.backupEnrollment])[0]?.backupAlias,
    'paper',
  );
  assert.equal(
    decodeBackupRevocation(fixture.backupRevocation).userChainSequence,
    16,
  );
  assert.equal(
    decodePairingOffer(fixture.pairingOffer).accountAlias,
    'personal',
  );
  assert.match(decodePairingOffer(fixture.pairingOffer).phrase, /\S/);
  assert.equal(
    decodeDeviceProvision(fixture.deviceProvision).userChainSequence,
    14,
  );
  assert.equal(
    decodeDeviceRemoval(fixture.deviceRemoval).userChainSequence,
    15,
  );
  assert.equal(decodePassphraseReport(fixture.passphraseReport).verified, true);
  assert.equal(decodeResetPreview(fixture.resetPreview).token.length > 0, true);
  assert.equal(decodeYubiCards([fixture.yubiCard]).length, 1);
  assert.equal(
    decodeYubiAccounts([fixture.yubiEnrollment])[0]?.state,
    'complete',
  );
  assert.equal(
    decodeYubiResult('create_yubi_account', fixture.yubiAccount).username,
    'satoshi',
  );
  assert.equal(
    decodeYubiResult('sync_yubi_account', fixture.yubiSync).entries,
    7,
  );
  assert.equal(
    decodeYubiResult('yubi_pin_status', fixture.yubiPinStatus).remaining,
    3,
  );
  assert.equal(
    decodeYubiResult('change_yubi_pin', fixture.yubiPinStatus).blocked,
    false,
  );
  assert.equal(
    decodeYubiResult('unblock_yubi_pin', fixture.yubiPinStatus).remaining,
    3,
  );
  assert.equal(
    decodeYubiResult('set_yubi_passphrase', fixture.passphraseReport).verified,
    true,
  );
  assert.equal(
    decodeYubiResult('change_yubi_passphrase', fixture.passphraseReport)
      .generation,
    2,
  );
  assert.equal(
    decodeYubiResult('verify_yubi_passphrase', fixture.passphraseReport)
      .verified,
    true,
  );
  assert.equal(
    decodeYubiResult('change_yubi_puk', fixture.yubiChanged).changed,
    true,
  );
  assert.equal(
    decodeYubiResult('rotate_yubi_management_key', fixture.yubiLifecycle)
      .managementGeneration,
    4,
  );
  assert.equal(
    decodeYubiResult('resume_yubi_management_key', fixture.yubiLifecycle)
      .managementEnrolled,
    true,
  );
  assert.equal(
    decodeYubiResult('recover_yubi_management_key', fixture.yubiLifecycle)
      .alias,
    'work_key',
  );
  assert.equal(
    decodeYubiResult('recover_yubi_subkey', fixture.yubiSubkeyRecovery)
      .certificateCount,
    2,
  );
  assert.equal(
    decodeYubiResult('revoke_yubi_device', fixture.yubiRevocation)
      .removedLocalCredential,
    true,
  );
});

test('decodeGoProfileDiscovery validates profile candidate fields and rejects invalid candidates', () => {
  const candidate = {
    candidateId: 'a'.repeat(64),
    username: 'satoshi',
    serverHint: 'foks.app',
    hostId: `02${'b'.repeat(64)}`,
    userId: `01${'c'.repeat(64)}`,
    deviceId: `04${'d'.repeat(64)}`,
    role: 'owner',
    storageKind: 'macos-keychain',
    hidden: false,
    provisional: false,
    pairable: true,
    copyable: true,
  };
  assert.deepEqual(
    decodeGoProfileDiscovery({ installed: true, candidates: [candidate] }),
    { installed: true, candidates: [candidate] },
  );
  assert.throws(() =>
    decodeGoProfileDiscovery({
      installed: true,
      candidates: [{ ...candidate, candidateId: 'A'.repeat(64) }],
    }),
  );
  assert.throws(() =>
    decodeGoProfileDiscovery({
      installed: true,
      candidates: [{ ...candidate, storageKind: 'passphrase', copyable: true }],
    }),
  );
  assert.throws(() =>
    decodeGoProfileDiscovery({ installed: false, candidates: [candidate] }),
  );
});

test('decoders reject invalid server status, malformed reset tokens, and invalid entity IDs', () => {
  assert.throws(
    () =>
      decodeServerStatus({
        profile: 'p',
        configuredProbe: 'x',
        host: {
          lookupName: 'x',
          canonicalName: 'x',
          hostId: 'short',
          chain: 1,
          epoch: 2,
        },
        leaseRequired: false,
        leaseExpiresAt: null,
        chatAvailable: false,
      }),
    /canonical 02 entity id/,
  );
  assert.throws(
    () =>
      decodeServerStatus({
        profile: 'p',
        configuredProbe: 'x',
        host: null,
        leaseExpiresAt: null,
        chatAvailable: false,
      }),
    /leaseRequired must be a boolean/,
  );
  assert.throws(
    () =>
      decodeServerStatus({
        profile: 'p',
        configuredProbe: 'x',
        host: null,
        leaseRequired: false,
        leaseExpiresAt: 100,
        chatAvailable: false,
      }),
    /protocol that does not use leases/,
  );
  assert.throws(
    () =>
      decodeCheckedServer({
        profile: 'p',
        acceptance: 'same',
        lookupName: 'x',
        canonicalName: 'x',
        hostId: `02${'1'.repeat(64)}`,
        chain: 1,
        epoch: 2,
      }),
    /inserted, advanced, or unchanged/,
  );
  assert.throws(
    () =>
      decodeResetPreview({
        profile: 'p',
        resumables: [],
        artifacts: [],
        token: 7,
        expiresInSeconds: 60,
      }),
    /token must be a string/,
  );
  assert.throws(
    () => decodeAccountDevices([{ id: 'd', role: 'superuser', current: true }]),
    /role is invalid/,
  );
  assert.equal(
    decodeAccountDevices([
      { id: `08${'8'.repeat(66)}`, role: 'owner', current: false },
    ])[0]?.id.startsWith('08'),
    true,
  );
  assert.throws(
    () =>
      decodeAccountDevices([
        { id: `08${'8'.repeat(64)}`, role: 'owner', current: false },
      ]),
    /software-device or YubiKey id/,
  );
  assert.throws(
    () =>
      decodeDeviceRemoval({
        deviceId: `08${'8'.repeat(66)}`,
        userChainSequence: 1,
        alreadyAbsent: false,
      }),
    /canonical 04 entity id/,
  );
  assert.deepEqual(
    decodeBackupRevocation({
      backupAlias: 'paper',
      accountAlias: 'personal',
      backupId: `10${'4'.repeat(64)}`,
      userChainSequence: 2,
      alreadyAbsent: false,
      removedLocalEnrollment: true,
    }),
    {
      backupAlias: 'paper',
      accountAlias: 'personal',
      backupId: `10${'4'.repeat(64)}`,
      userChainSequence: 2,
      alreadyAbsent: false,
      removedLocalEnrollment: true,
    },
  );
  assert.throws(
    () =>
      decodeBackupRevocation({
        backupAlias: 'paper',
        accountAlias: 'personal',
        backupId: `10${'4'.repeat(64)}`,
        userChainSequence: 2,
        alreadyAbsent: false,
        removedLocalEnrollment: false,
      }),
    /removedLocalEnrollment must be true/,
  );
  assert.throws(
    () => decodeYubiAccounts([{ alias: 'key', state: 'unknown' }]),
    /state is invalid/,
  );
  assert.throws(
    () =>
      decodeYubiResult('create_yubi_account', {
        alias: 'key',
        username: 'satoshi',
        yubiId: `08${'8'.repeat(64)}`,
        subkeyId: `0d${'d'.repeat(64)}`,
        userChainSequence: 1,
        managementEnrolled: true,
      }),
    /canonical YubiKey id/,
  );
});

test('first-run response decoders validate profile checks, pending operations, and group discovery', () => {
  assert.equal(
    decodeCheckedProfile({
      profile: 'work',
      acceptance: 'advanced',
      lookupName: 'foks.example',
      canonicalName: 'foks.example',
      hostId: `02${'1'.repeat(64)}`,
      chain: 1,
      epoch: 1,
    }).acceptance,
    'advanced',
  );
  assert.equal(
    decodeCheckedProfile({
      profile: 'work',
      acceptance: 'unchanged',
      lookupName: 'foks.example',
      canonicalName: 'foks.example',
      hostId: `02${'1'.repeat(64)}`,
      chain: 1,
      epoch: 1,
    }).acceptance,
    'unchanged',
  );
  assert.throws(
    () =>
      decodeCheckedProfile({
        profile: 'work',
        acceptance: 'invented',
        lookupName: 'foks.example',
        canonicalName: 'foks.example',
        hostId: `02${'1'.repeat(64)}`,
        chain: 1,
        epoch: 1,
      }),
    /acceptance is invalid/,
  );
  assert.throws(
    () =>
      decodeCheckedProfile({
        profile: 'work',
        acceptance: 'inserted',
        lookupName: 'foks.example',
        canonicalName: 'foks.example',
        hostId: `03${'1'.repeat(64)}`,
        chain: 1,
        epoch: 1,
      }),
    /canonical 02 entity id/,
  );
  assert.throws(
    () => decodePendingOperations([{ kind: 'invented', alias: 'work' }]),
    /pending-operation kind/,
  );
  assert.throws(
    () =>
      decodeGroupDiscovery({
        accountAlias: 'personal',
        groups: [
          {
            alias: 'ops',
            accountAlias: 'other',
            teamIdHex: `03${'1'.repeat(64)}`,
            kind: 'named',
            name: 'Ops',
            active: true,
          },
        ],
      }),
    /different account/,
  );
  assert.throws(
    () =>
      decodeGroupDiscovery({
        accountAlias: 'personal',
        groups: [
          {
            alias: 'ops',
            accountAlias: 'personal',
            teamIdHex: `14${'1'.repeat(64)}`,
            kind: 'adhoc',
            name: 'Ops',
            active: true,
          },
        ],
      }),
    /name must be present only/,
  );
  assert.throws(
    () =>
      decodeGroupDiscovery({
        accountAlias: 'personal',
        groups: [
          {
            alias: 'ops',
            accountAlias: 'personal',
            teamIdHex: `14${'1'.repeat(64)}`,
            kind: 'named',
            name: 'Ops',
            active: true,
          },
        ],
      }),
    /canonical 03 entity id/,
  );
});

test('decodeCatalog preserves structured failure objects and typed error details', () => {
  const decoded = decodeCatalog({
    ...catalog,
    failures: [
      {
        scope: 'profile',
        profile: 'foks.example.net',
        source: 'KV catalog',
        error: {
          code: 'capability-denied',
          message: 'KV unavailable',
          retryable: false,
          ambiguous: false,
          fatal: true,
          details: { capability: 'kv', profile: 'foks.example.net' },
        },
      },
    ],
  });
  assert.equal(decoded.failures[0]?.error.details?.capability, 'kv');
  assert.throws(
    () =>
      decodeCatalog({
        ...catalog,
        failures: [
          {
            scope: 'profile',
            profile: 'x',
            error: {
              code: 'failure',
              message: 'failed',
              retryable: false,
              ambiguous: false,
              fatal: true,
            },
          },
        ],
      }),
    /source must be a string/,
  );
});

test('normalizeCommandError preserves typed error fields and defaults malformed errors to fatal', () => {
  assert.deepEqual(
    normalizeCommandError({
      code: 'lease-lapsed',
      message: 'Connection session expired. Vault access is paused.',
      retryable: true,
      ambiguous: false,
      fatal: false,
    }),
    {
      code: 'lease-lapsed',
      message: 'Connection session expired. Vault access is paused.',
      retryable: true,
      ambiguous: false,
      fatal: false,
    },
  );
  assert.equal(normalizeCommandError('oops').fatal, true);
});

test('passive server status uses structured schema codes, not wording', () => {
  assert.equal(
    shouldReportPassiveServerStatusError({
      code: 'unsupported-schema',
      message: 'wording may change without changing classification',
      retryable: false,
      ambiguous: false,
      fatal: true,
      details: { foundSchema: 23, supportedSchema: 27 },
    }),
    false,
  );
  assert.equal(
    shouldReportPassiveServerStatusError({
      code: 'operation-failed',
      message:
        'system store is out of date (v23), could not auto-update to current version (v27)',
      retryable: false,
      ambiguous: false,
      fatal: true,
    }),
    true,
  );
});

test('agent status decoding preserves bootstrap as a non-ready variant', () => {
  assert.deepEqual(decodeAgentStatus({ state: 'ready' }), { state: 'ready' });
  assert.deepEqual(
    decodeAgentStatus({ state: 'bootstrap', step: 'initialize-state' }),
    { state: 'bootstrap', step: 'initialize-state' },
  );
});

test('loadSnapshot never requests the catalog while the agent requires bootstrap', async () => {
  let catalogRequests = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    agentStatus: async () => ({
      state: 'bootstrap',
      step: 'initialize-state',
    }),
    listCatalog: async () => {
      catalogRequests++;
      return catalog;
    },
  };
  await assert.rejects(loadSnapshot(bridge), (error: unknown) => {
    assert.equal(normalizeCommandError(error).code, 'bootstrap-required');
    return true;
  });
  assert.equal(catalogRequests, 0);
});

test('loadSnapshot binds later reads to the catalog generation and retries once when it changes', async () => {
  const base = mockBridge(FIXTURE);
  const seen: { servers: unknown[]; accounts: unknown[] } = {
    servers: [],
    accounts: [],
  };
  let loads = 0;
  const bridge: Bridge = {
    ...base,
    listCatalog: async () => {
      loads++;
      return { ...(await base.listCatalog()), generation: loads };
    },
    listServers: async (generation) => {
      seen.servers.push(generation);
      // The first sequence loses its snapshot to a concurrent load.
      if (generation === 1)
        throw {
          code: 'catalog-required',
          message: 'The vault changed while it was loading.',
          retryable: true,
          ambiguous: false,
          fatal: false,
        };
      return base.listServers();
    },
    listAccounts: async (generation) => {
      seen.accounts.push(generation);
      return base.listAccounts();
    },
  };
  const snapshot = await loadSnapshot(bridge);
  assert.equal(
    loads,
    2,
    'exactly one further attempt after the snapshot changed',
  );
  assert.deepEqual(seen.servers, [1, 2]);
  assert.deepEqual(seen.accounts, [2]);
  assert.equal(snapshot.profileInventoryStatus, 'complete');
  // A second change is reported rather than retried indefinitely.
  const failing: Bridge = {
    ...base,
    listAccounts: async () => {
      throw {
        code: 'catalog-required',
        message: 'The vault changed while it was loading.',
        retryable: true,
        ambiguous: false,
        fatal: false,
      };
    },
  };
  await assert.rejects(loadSnapshot(failing), (error: unknown) => {
    assert.equal(normalizeCommandError(error).code, 'catalog-required');
    return true;
  });
});

test('loadSnapshot propagates catalog-wide readiness failures to the lifecycle owner', async () => {
  const readiness: string[] = [];
  const unlisten = onAgentReadinessRequired((error) => {
    readiness.push(error.code);
  });
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    listCatalog: async () => ({
      ...catalog,
      failures: [
        {
          scope: 'profile',
          profile: 'foks.example.net',
          source: 'catalog',
          error: {
            code: 'bootstrap-required',
            message: 'Agent state changed while loading the catalog.',
            retryable: false,
            ambiguous: false,
            fatal: false,
          },
        },
      ],
    }),
  };
  try {
    await assert.rejects(loadSnapshot(bridge), (error: unknown) => {
      assert.equal(normalizeCommandError(error).code, 'bootstrap-required');
      return true;
    });
    assert.deepEqual(readiness, ['bootstrap-required']);
  } finally {
    unlisten();
  }
});

test('loadSnapshot reports native agent loss once without mislabeling it as lease failure', async () => {
  const previousWindow = globalThis.window;
  Object.defineProperty(globalThis, 'window', {
    value: {
      __TAURI_INTERNALS__: {
        invoke: async () => {
          throw {
            code: 'agent-lost',
            message: 'Agent connection closed.',
            retryable: true,
            ambiguous: false,
            fatal: false,
          };
        },
      },
    },
    configurable: true,
  });
  const readiness: string[] = [];
  const unlisten = onAgentReadinessRequired((error) => {
    readiness.push(error.code);
  });
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    listCatalog: async () => catalog,
    listServers: async () => [listedServer('foks.example.net', 'ok')],
    describeServerStatus: (profile) =>
      tauriBridge.describeServerStatus(profile),
  };
  try {
    await assert.rejects(loadSnapshot(bridge), (error: unknown) => {
      assert.equal(normalizeCommandError(error).code, 'agent-lost');
      return true;
    });
    assert.deepEqual(readiness, ['agent-lost']);
  } finally {
    unlisten();
    if (previousWindow === undefined)
      delete (globalThis as { window?: Window }).window;
    else
      Object.defineProperty(globalThis, 'window', {
        value: previousWindow,
        configurable: true,
      });
  }
});

test('selectBridge returns native tauriBridge when window.__TAURI_INTERNALS__ is present', async () => {
  const previous = globalThis.window;
  Object.defineProperty(globalThis, 'window', {
    value: { __TAURI_INTERNALS__: {} },
    configurable: true,
  });
  try {
    assert.equal(await selectBridge(), tauriBridge);
  } finally {
    if (previous === undefined)
      delete (globalThis as { window?: Window }).window;
    else
      Object.defineProperty(globalThis, 'window', {
        value: previous,
        configurable: true,
      });
  }
});

test('loadSnapshot retains validated account-store identities in the mock snapshot', async () => {
  const snapshot = await loadSnapshot(mockBridge(FIXTURE), FIXTURE);
  assert.equal(snapshot.accounts.length, FIXTURE.accounts.length);
  for (const account of snapshot.accounts) {
    assert.ok(account.store);
    assert.equal(
      snapshot.stores.find((store) => store.id === account.store)?.kind,
      'account',
    );
  }
});

test('discoverUnboundTeams discovers teams only for accounts with no binding', async () => {
  const calls: { profile: string; alias: string }[] = [];
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    discoverGroups: async (profile, accountAlias) => {
      calls.push({ profile, alias: accountAlias });
      return {
        accountAlias,
        groups: [
          {
            alias: 'discovered',
            accountAlias,
            teamIdHex: 'aa'.repeat(33),
            kind: 'named',
            name: 'Discovered',
            active: true,
          },
        ],
      };
    },
  };
  const snapshot: AgentSnapshot = {
    ...FIXTURE,
    accounts: [
      {
        store: 'acct:personal',
        alias: 'personal',
        username: 'satoshi',
        server: 'foks.example.net',
      },
      {
        store: 'acct:work',
        alias: 'work',
        username: 'vitalik',
        server: 'acme',
      },
    ],
    stores: [
      {
        id: 'acct:work',
        kind: 'account',
        name: 'Work',
        server: 'acme',
        account: 'work',
      },
      {
        id: 'team:eng',
        kind: 'team',
        name: 'Engineering',
        alias: 'eng',
        server: 'acme',
        account: 'work',
        active: true,
        team_kind: 'named',
        team_id_hex: 'bb'.repeat(33),
      },
    ],
  };
  assert.equal(await discoverUnboundTeams(bridge, snapshot), true);
  assert.deepEqual(calls, [{ profile: 'foks.example.net', alias: 'personal' }]);
});

test('startup discovery requests a catalog reload even for empty or failed discovery', async () => {
  const snapshot: AgentSnapshot = {
    ...FIXTURE,
    stores: FIXTURE.stores.filter((store) => store.kind !== 'team'),
    accounts: FIXTURE.accounts.slice(0, 1),
  };
  assert.equal(snapshot.accounts.length, 1);
  for (const failed of [false, true]) {
    let catalogValid = true;
    const bridge: Bridge = {
      ...mockBridge(snapshot),
      discoverGroups: async (_profile, accountAlias) => {
        // The native mutation command invalidates its catalog before dispatch.
        catalogValid = false;
        if (failed)
          throw {
            code: 'ambiguous',
            message: 'Timed out',
            retryable: false,
            ambiguous: true,
          };
        return { accountAlias, groups: [] };
      },
    };
    assert.equal(await discoverUnboundTeams(bridge, snapshot), true);
    assert.equal(catalogValid, false);
  }
});

test('discoverUnboundTeams leaves a fully bound snapshot untouched', async () => {
  let calls = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    discoverGroups: async (_profile, accountAlias) => {
      calls += 1;
      return { accountAlias, groups: [] };
    },
  };
  assert.equal(await discoverUnboundTeams(bridge, FIXTURE), false);
  assert.equal(calls, 0);
});

test('loadSnapshot makes a single catalog call and does not leak fixture data in native mode', async () => {
  let calls = 0;
  const accountStore = catalog.stores[0];
  assert.ok(accountStore);
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    agentStatus: async () => ({ state: 'ready' }),
    listCatalog: async () => {
      calls += 1;
      return catalog;
    },
    listStores: async () => ({ ...catalog, items: [] }),
    listServers: async () => [
      listedServer('foks.example.net', 'never-probed', ['satoshi']),
    ],
    describeServerStatus: async () => ({
      profile: 'foks.example.net',
      configuredProbe: 'foks.example.net',
      host: checkedHost,
      leaseRequired: true,
      leaseExpiresAt: 2_000_000_000,
      chatAvailable: true,
    }),
    listAccounts: async () => [
      {
        store: accountStore.id,
        server: 'foks.example.net',
        alias: 'satoshi',
        username: 'satoshi',
      },
    ],
    listParties: async () => [],
    listFederation: async () => [],
    readItem: async () => {
      throw new Error('not used');
    },
    copyItemValue: async () => ({ ok: true }),
    copyItemPath: async () => ({ ok: true }),
    downloadFile: async () => ({ saved: false }),
  };
  const snapshot = await loadSnapshot(bridge, FIXTURE);
  assert.equal(calls, 1);
  assert.deepEqual(snapshot.stores, catalog.stores);
  assert.deepEqual(snapshot.accounts, [
    {
      store: accountStore.id,
      server: 'foks.example.net',
      alias: 'satoshi',
      username: 'satoshi',
    },
  ]);
  assert.equal(snapshot.parties.length, 0);
  assert.equal(snapshot.notifications.length, 0);
  assert.equal(
    snapshot.plaintext['acct:personal|/logins/github.com'],
    undefined,
  );
});

test('loadSnapshot keeps known stores visible while revoking access to unavailable stores', async () => {
  const known = catalog.stores[0];
  assert.ok(known);
  const response: CatalogDto = {
    ...catalog,
    stores: [],
    knownStores: [known],
    inventory: [
      {
        profile: 'foks.example.net',
        accountsComplete: false,
        teamsComplete: true,
      },
    ],
    items: [],
    failures: [
      {
        scope: 'profile',
        profile: 'foks.example.net',
        source: 'account stores',
        error: {
          code: 'transport',
          message: 'offline',
          retryable: true,
          ambiguous: false,
          fatal: false,
        },
      },
    ],
  };
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    agentStatus: async () => ({ state: 'ready' }),
    listCatalog: async () => response,
    listServers: async () => [
      listedServer('foks.example.net', 'never-probed', ['satoshi']),
    ],
    describeServerStatus: async () => ({
      profile: 'foks.example.net',
      configuredProbe: 'foks.example.net',
      host: checkedHost,
      leaseRequired: true,
      leaseExpiresAt: 2_000_000_000,
      chatAvailable: true,
    }),
    listAccounts: async () => [],
    listParties: async () => [],
    listFederation: async () => [],
  };
  const snapshot = await loadSnapshot(bridge);
  assert.deepEqual(
    snapshot.stores.map((store) => store.id),
    [known.id],
  );
  assert.deepEqual(snapshot.storeInventory, [
    {
      store: known.id,
      status: 'unavailable',
      restrictions: [],
    },
  ]);
  assert.equal(snapshot.profileInventory[0]?.accounts, 'unavailable');
  assert.equal(storeReadable(snapshot, known.id), false);
  assert.equal(canCreateInStore(snapshot, known.id), false);
});

test('creates a notification when a server cannot be described instead of omitting it', async () => {
  const accountStore = catalog.stores[0];
  assert.ok(accountStore);
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    agentStatus: async () => ({ state: 'ready' }),
    listCatalog: async () => {
      return catalog;
    },
    listStores: async () => ({ ...catalog, items: [] }),
    listServers: async () => [
      listedServer('foks.example.net', 'never-probed', ['satoshi']),
    ],
    // Verify that an unprobed server with null host raises a never-probed notification.
    describeServerStatus: async () => ({
      profile: 'foks.example.net',
      configuredProbe: 'foks.example.net',
      host: null,
      leaseRequired: false,
      leaseExpiresAt: null,
      chatAvailable: false,
    }),
    listAccounts: async () => [
      {
        store: accountStore.id,
        server: 'foks.example.net',
        alias: 'satoshi',
        username: 'satoshi',
      },
    ],
    listParties: async () => [],
    listFederation: async () => [],
    readItem: async () => {
      throw new Error('not used');
    },
    copyItemValue: async () => ({ ok: true }),
    copyItemPath: async () => ({ ok: true }),
    downloadFile: async () => ({ saved: false }),
  };
  const snapshot = await loadSnapshot(bridge, FIXTURE);
  const note = snapshot.notifications.find((entry) =>
    entry.id.startsWith('verification-required-'),
  );
  assert.ok(
    note,
    `a never-probed note: ${JSON.stringify(snapshot.notifications.map((n) => n.id))}`,
  );
  assert.equal(note.title, 'foks.example.net is locked');
  assert.equal(note.action, 'Verify');
});

test('loadSnapshot does not fetch team rosters for blocked profiles', async () => {
  const storeId = '{"Team":{"profile":"foks.example.net","team_alias":"ops"}}';
  const blocked: CatalogDto = {
    profiles: ['foks.example.net'],
    stores: [
      {
        id: storeId,
        kind: 'team',
        name: 'Ops',
        server: 'foks.example.net',
        account: 'satoshi',
        alias: 'ops',
        active: true,
        team_kind: 'named',
        team_id_hex: 'aa',
      },
    ],
    knownStores: [],
    inventory: [
      {
        profile: 'foks.example.net',
        accountsComplete: true,
        teamsComplete: true,
      },
    ],
    items: [],
    failures: [],
    blockedProfiles: ['foks.example.net'],
  };
  let rosterCalls = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    agentStatus: async () => ({ state: 'ready' }),
    listCatalog: async () => blocked,
    listStores: async () => ({ ...blocked, items: [] }),
    listServers: async () => [listedServer('foks.example.net', 'blocked')],
    listAccounts: async () => [],
    listParties: async () => {
      rosterCalls += 1;
      return [];
    },
    listFederation: async () => {
      rosterCalls += 1;
      return [];
    },
    readItem: async () => {
      throw new Error('not used');
    },
    copyItemValue: async () => ({ ok: true }),
    copyItemPath: async () => ({ ok: true }),
    downloadFile: async () => ({ saved: false }),
  };
  const snapshot = await loadSnapshot(bridge);
  assert.equal(rosterCalls, 0);
  assert.equal(snapshot.notifications[0]?.title, 'foks.example.net is locked');
});

test('loadSnapshot does not fetch members for an inactive team', async () => {
  const storeId = '{"Team":{"profile":"foks.example.net","team_alias":"ops"}}';
  const accountId =
    '{"Account":{"profile":"foks.example.net","account_alias":"satoshi"}}';
  const pending: CatalogDto = {
    profiles: ['foks.example.net'],
    stores: [
      {
        id: accountId,
        kind: 'account',
        name: 'Personal',
        server: 'foks.example.net',
        account: 'satoshi',
      },
      {
        id: storeId,
        kind: 'team',
        name: 'Ops',
        server: 'foks.example.net',
        account: 'satoshi',
        alias: 'ops',
        active: false,
        team_kind: 'named',
        team_id_hex: 'aa',
      },
    ],
    knownStores: [],
    inventory: [
      {
        profile: 'foks.example.net',
        accountsComplete: true,
        teamsComplete: true,
      },
    ],
    items: [],
    failures: [],
    blockedProfiles: [],
  };
  let rosterCalls = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    agentStatus: async () => ({ state: 'ready' }),
    listCatalog: async () => pending,
    listStores: async () => ({ ...pending, items: [] }),
    listServers: async () => [
      listedServer('foks.example.net', 'never-probed', ['satoshi']),
    ],
    describeServerStatus: async () => ({
      profile: 'foks.example.net',
      configuredProbe: 'foks.example.net',
      host: checkedHost,
      leaseRequired: true,
      leaseExpiresAt: 2_000_000_000,
      chatAvailable: true,
    }),
    listAccounts: async () => [
      {
        store: accountId,
        server: 'foks.example.net',
        alias: 'satoshi',
        username: 'satoshi',
      },
    ],
    listParties: async () => {
      rosterCalls += 1;
      throw new Error(
        'FOKS account record is invalid: team creation is still pending',
      );
    },
    listFederation: async () => {
      rosterCalls += 1;
      throw new Error(
        'FOKS account record is invalid: team creation is still pending',
      );
    },
    readItem: async () => {
      throw new Error('not used');
    },
    copyItemValue: async () => ({ ok: true }),
    copyItemPath: async () => ({ ok: true }),
    downloadFile: async () => ({ saved: false }),
  };
  const snapshot = await loadSnapshot(bridge);
  assert.equal(rosterCalls, 0);
  const ops = snapshot.stores.find((store) => store.id === storeId);
  assert.equal(ops?.kind, 'team');
  assert.equal(ops?.kind === 'team' && ops.active, false);
  assert.equal(
    snapshot.parties.some((party) => party.store === storeId),
    false,
  );
});

test('loadSnapshot evaluates store access based on server lease validity and protocol requirements', async () => {
  assert.equal(signedLeaseState(101, 100), 'fresh');
  assert.equal(signedLeaseState(100, 100), 'lapsed');
  assert.equal(signedLeaseState(null, 100), 'unavailable');
  const profiles = ['fresh', 'expired', 'missing', 'v019', 'failed'];
  const stores = profiles.map((profile) => ({
    id: `account:${profile}`,
    kind: 'account' as const,
    name: profile,
    server: profile,
    account: profile,
  }));
  const response: CatalogDto = {
    profiles,
    stores,
    knownStores: stores,
    inventory: profiles.map((profile) => ({
      profile,
      accountsComplete: true,
      teamsComplete: true,
    })),
    items: stores.map((store) => ({
      store: store.id,
      path: '/value',
      kind: 'Secret' as const,
      size: 1,
      version: 1,
      read: { role: 'Owner' as const },
      write: { role: 'Owner' as const },
    })),
    failures: [],
    blockedProfiles: [],
  };
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base,
    native: true,
    listCatalog: async () => response,
    listServers: async () =>
      profiles.map((profile) => ({
        ...listedServer(profile, 'never-probed', [profile]),
        name: `${profile}.example`,
      })),
    describeServerStatus: async (profile) => {
      if (profile === 'failed')
        throw new Error('failed to retrieve server status');
      return {
        profile,
        configuredProbe: `${profile}.example`,
        host: checkedHost,
        leaseRequired: profile !== 'v019',
        leaseExpiresAt:
          profile === 'fresh' ? 200 : profile === 'expired' ? 99 : null,
        chatAvailable: profile === 'fresh' || profile === 'v019',
      };
    },
    listAccounts: async () =>
      stores
        .filter((store) => store.server === 'fresh' || store.server === 'v019')
        .map((store) => ({
          store: store.id,
          server: store.server,
          alias: store.account,
          username: store.account,
        })),
    listParties: async () => [],
    listFederation: async () => [],
  };
  const snapshot = await loadSnapshot(bridge, undefined, 100);
  assert.deepEqual(
    snapshot.items.map((item) => item.store),
    ['account:fresh', 'account:v019'],
  );
  assert.deepEqual(
    snapshot.accounts.map((account) => account.store),
    ['account:fresh', 'account:v019'],
  );
  assert.deepEqual(
    snapshot.stores.map((store) => store.id),
    stores.map((store) => store.id),
    'local aliases remain visible while unverified server contents are hidden',
  );
  assert.equal(
    serverAvailability(
      snapshot,
      snapshot.servers.find((server) => server.id === 'fresh')!,
      { nowSeconds: 100 },
    ).available,
    true,
  );
  const expired = serverAvailability(
    snapshot,
    snapshot.servers.find((server) => server.id === 'expired')!,
    { nowSeconds: 100 },
  );
  assert.equal(
    expired.available ? 'available' : expired.reason,
    'check-in-expired',
  );
  assert.equal(
    snapshot.servers.find((server) => server.id === 'missing')?.compatibility
      .status,
    'required-unavailable',
  );
  assert.equal(
    snapshot.servers.find((server) => server.id === 'v019')?.compatibility
      .status,
    'not-required',
  );
  assert.equal(
    snapshot.servers.find((server) => server.id === 'failed')?.passiveStatus
      .status,
    'failed',
  );
  assert.deepEqual(snapshot.observedExpiredLeases, []);
  assert.match(
    snapshot.notifications.find(
      (note) => note.id === 'status-unavailable-failed',
    )?.detail ?? '',
    /Server contents are unavailable until the connection status is verified./,
  );

  const racedSnapshot = await loadSnapshot(
    {
      ...bridge,
      listAccounts: async () =>
        stores.map((store) => ({
          store: store.id,
          server: store.server,
          alias: store.account,
          username: store.account,
        })),
    },
    undefined,
    100,
  );
  assert.deepEqual(
    racedSnapshot.accounts.map((account) => account.store),
    ['account:fresh', 'account:v019'],
    'stale account rows are discarded without affecting active server accounts',
  );
});

test('loadSnapshot only imports accounts belonging to active profiles', async () => {
  const stores = FIXTURE.stores.filter(
    (store) =>
      store.id === 'acct:personal' ||
      store.id === 'acct:work' ||
      store.id === 'team:eng',
  );
  const response: CatalogDto = {
    profiles: ['personal', 'acme'],
    stores,
    knownStores: stores,
    inventory: [
      { profile: 'personal', accountsComplete: true, teamsComplete: true },
      { profile: 'acme', accountsComplete: true, teamsComplete: true },
    ],
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
    listAccounts: async () => [
      {
        store: 'acct:personal',
        server: 'personal',
        alias: 'personal',
        username: 'satoshi',
      },
      {
        store: 'acct:work',
        server: 'acme',
        alias: 'work',
        username: 'vitalik',
      },
    ],
    listParties: async () => {
      rosterCalls += 1;
      return [];
    },
    listFederation: async () => {
      rosterCalls += 1;
      return [];
    },
  };
  const snapshot = await loadSnapshot(bridge);
  assert.deepEqual(
    snapshot.accounts.map((account) => account.store),
    ['acct:personal'],
  );
  assert.equal(rosterCalls, 0);
});

test('loadSnapshot omits rosters for lapsed servers and enriches active member and team labels', async () => {
  const base = mockBridge(FIXTURE);
  let rosterCalls = 0;
  const bridge: Bridge = {
    ...base,
    native: true,
    listCatalog: async () => ({
      profiles: ['personal', 'acme'],
      stores: [...FIXTURE.stores],
      knownStores: [...FIXTURE.stores],
      inventory: [
        { profile: 'personal', accountsComplete: true, teamsComplete: true },
        { profile: 'acme', accountsComplete: true, teamsComplete: true },
      ],
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
      {
        store: 'acct:personal',
        alias: 'personal',
        username: 'satoshi',
        server: 'personal',
      },
    ],
    listServers: async () =>
      FIXTURE.servers.map((server) =>
        server.id === 'acme'
          ? { ...server, lease: null, state: 'lease-lapsed' as const }
          : { ...server, lease: null },
      ),
    listGroupDetails: async (storeId) => {
      rosterCalls += 1;
      return base.listGroupDetails(storeId);
    },
  };
  const snapshot = await loadSnapshot(bridge);
  assert.equal(
    rosterCalls,
    1,
    'only active group rosters are requested; inactive and lapsed groups are skipped',
  );
  assert.equal(
    snapshot.parties.some((party) => party.store === 'team:eng'),
    false,
  );

  const freshBridge = {
    ...bridge,
    listServers: async () =>
      FIXTURE.servers.map((server) => ({
        ...server,
        lease: null,
        state: 'never-probed' as const,
        chat_available: false,
      })),
    listAccounts: async () => [
      {
        store: 'acct:personal',
        alias: 'personal',
        username: 'satoshi',
        server: 'personal',
      },
      {
        store: 'acct:work',
        alias: 'work',
        username: 'vitalik',
        server: 'acme',
      },
    ],
    describeServerStatus: async (profile: string) => ({
      profile,
      configuredProbe: profile,
      host:
        profile === 'partner'
          ? null
          : { ...checkedHost, lookupName: profile, canonicalName: profile },
      leaseRequired: true,
      leaseExpiresAt: profile === 'partner' ? null : 2_000_000_000,
      chatAvailable: profile !== 'partner',
    }),
    listGroupDetails: async (storeId: string) => {
      const current = await bridge.listGroupDetails(storeId);
      if (storeId !== 'team:eng' || current.parties.status !== 'success')
        return current;
      const local = current.parties.value.find(
        (party) => party.username === 'vitalik',
      );
      assert.ok(local);
      return {
        ...current,
        parties: {
          status: 'success' as const,
          value: [
            ...current.parties.value,
            {
              ...local,
              label: 'you',
              locally_manageable: false,
              scoped_host_id_hex: 'remote-host',
              party_id_hex: `01${'9'.repeat(64)}`,
            },
          ],
        },
      };
    },
  };
  const fresh = await loadSnapshot(freshBridge);
  const sameNames = fresh.parties.filter(
    (party) => party.username === 'vitalik',
  );
  assert.equal(
    sameNames.find((party) => party.locally_manageable)?.label,
    'you',
  );
  assert.equal(
    sameNames.find((party) => !party.locally_manageable)?.label,
    undefined,
  );
  assert.equal(
    fresh.parties.find((party) => party.party_kind === 'named-team')?.team_name,
    'homelab @ foks.example.net',
  );
});

test('loadSnapshot retains stores and items when group details encounter transient server errors', async () => {
  const base = mockBridge(FIXTURE);
  const failure = {
    code: 'rate-limited',
    message: 'The FOKS server is busy.',
    retryable: true,
    ambiguous: false,
    fatal: false,
  };
  const bridge: Bridge = {
    ...base,
    native: true,
    fixtureSnapshot: undefined,
    listGroupDetails: async (storeId) =>
      storeId === 'team:household'
        ? {
            parties: { status: 'error', error: failure },
            federation: {
              status: 'error',
              error: { ...failure, code: 'quota-exceeded', retryable: false },
            },
          }
        : base.listGroupDetails(storeId),
  };
  const snapshot = await loadSnapshot(bridge);
  assert.ok(snapshot.stores.some((store) => store.id === 'team:eng'));
  assert.ok(snapshot.items.some((item) => item.store === 'team:household'));
  assert.deepEqual(snapshot.groupDetailFailures, [
    {
      store: 'team:household',
      source: 'roster',
      code: 'rate-limited',
      message: failure.message,
      retryable: true,
    },
    {
      store: 'team:household',
      source: 'federation',
      code: 'quota-exceeded',
      message: failure.message,
      retryable: false,
    },
  ]);
  assert.equal(
    snapshot.parties.some((party) => party.store === 'team:household'),
    false,
  );
  assert.equal(
    snapshot.notifications.filter((note) => note.id.startsWith('group-'))
      .length,
    2,
  );
});

test('loadSnapshot handles disabled group capabilities gracefully without leaking internal errors', async () => {
  const base = mockBridge(FIXTURE);
  const capabilityBridge: Bridge = {
    ...base,
    native: true,
    fixtureSnapshot: undefined,
    listGroupDetails: async (storeId) => {
      const details = await base.listGroupDetails(storeId);
      return storeId === 'team:household'
        ? {
            ...details,
            parties: {
              status: 'error',
              error: {
                code: 'capability-denied',
                message: 'Teams are disabled.',
                retryable: false,
                ambiguous: false,
                fatal: false,
                details: { capability: 'teams' },
              },
            } as const,
          }
        : details;
    },
  };
  const snapshot = await loadSnapshot(capabilityBridge);
  assert.deepEqual(
    snapshot.groupDetailFailures.filter(
      (failure) => failure.store === 'team:household',
    ),
    [
      {
        store: 'team:household',
        source: 'roster',
        code: 'capability-denied',
        message: 'Teams are disabled.',
        retryable: false,
      },
    ],
  );

  const malformedBridge: Bridge = {
    ...base,
    native: true,
    fixtureSnapshot: undefined,
    listGroupDetails: async () => {
      throw new Error('private implementation detail');
    },
  };
  await assert.rejects(
    loadSnapshot(malformedBridge),
    (error: unknown) =>
      error instanceof Error &&
      error.message.includes('unrecognized group-detail error') &&
      !error.message.includes('private implementation detail'),
  );
});

test('loadSnapshot rejects group detail payloads referencing mismatched store IDs', async () => {
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base,
    native: true,
    fixtureSnapshot: undefined,
    listGroupDetails: async (storeId) => {
      const details = await base.listGroupDetails(storeId);
      if (details.parties.status !== 'success' || !details.parties.value.length)
        return details;
      return {
        ...details,
        parties: {
          status: 'success',
          value: [{ ...details.parties.value[0], store: 'team:other' }],
        },
      };
    },
  };
  await assert.rejects(loadSnapshot(bridge), /roster for a different store/);
});

test('mock revokeOwnerBackup validates alias, removes enrollment, and behaves idempotently on retry', async () => {
  const bridge = mockBridge(FIXTURE);
  const store = FIXTURE.stores.find(
    (candidate) =>
      candidate.kind === 'account' && candidate.account === 'personal',
  );
  assert.ok(store);
  const [backup] = await bridge.listBackupEnrollments(store.id);
  assert.ok(backup);

  await assert.rejects(
    bridge.revokeOwnerBackup(store.id, backup, 'wrong'),
    (error: unknown) => normalizeCommandError(error).code === 'invalid-request',
  );
  await assert.rejects(
    bridge.revokeOwnerBackup(
      store.id,
      { ...backup, backupId: `10${'5'.repeat(64)}` },
      backup.backupAlias,
    ),
    (error: unknown) => normalizeCommandError(error).code === 'invalid-request',
  );
  assert.equal((await bridge.listBackupEnrollments(store.id)).length, 1);

  const revoked = await bridge.revokeOwnerBackup(
    store.id,
    backup,
    backup.backupAlias,
  );
  assert.equal(revoked.backupId, backup.backupId);
  assert.equal(revoked.alreadyAbsent, false);
  assert.equal((await bridge.listBackupEnrollments(store.id)).length, 0);

  const repeated = await bridge.revokeOwnerBackup(
    store.id,
    backup,
    backup.backupAlias,
  );
  assert.equal(repeated.alreadyAbsent, true);
  assert.equal(repeated.removedLocalEnrollment, true);
});

test('mock createTextItem adds item to catalog and allows reading by specific version', async () => {
  const bridge = mockBridge(FIXTURE);
  await bridge.createTextItem({
    storeId: 'acct:personal',
    path: '/agents/new-key',
    value: 'new value',
  });
  const created = (await bridge.listCatalog()).items.find(
    (item) => item.path === '/agents/new-key',
  );
  assert.ok(created);
  assert.equal(created.version, 1);
  assert.equal(
    (
      await bridge.readItem({
        storeId: created.store,
        path: created.path,
        version: created.version,
      })
    ).value,
    'new value',
  );
});

test('group item creation requires and preserves explicit read and write permissions', async () => {
  const bridge = mockBridge(FIXTURE);
  await bridge.createTextItem({
    storeId: 'team:eng',
    path: '/shared/text',
    value: 'value',
    readRole: 'Member:-32768',
    writeRole: 'Admin',
  });
  await bridge.createLink({
    storeId: 'team:eng',
    path: '/shared/link',
    target: '/shared/text',
    readRole: 'Admin',
    writeRole: 'Member:32767',
  });
  await bridge.importDroppedFile({
    storeId: 'team:eng',
    path: '/shared/file',
    sourcePath: '/tmp/file',
    readRole: 'Member:0',
    writeRole: 'Admin',
  });
  await bridge.createFolder({
    storeId: 'team:eng',
    path: '/shared/folder',
    readRole: 'Owner',
    writeRole: 'Owner',
  });
  const created = (await bridge.listCatalog()).items.filter(
    (item) => item.store === 'team:eng' && item.path.startsWith('/shared/'),
  );
  assert.deepEqual(
    created.map(({ path, read, write }) => ({ path, read, write })),
    [
      {
        path: '/shared/text',
        read: { role: 'Member', visibility: -32768 },
        write: { role: 'Admin' },
      },
      {
        path: '/shared/link',
        read: { role: 'Admin' },
        write: { role: 'Member', visibility: 32767 },
      },
      {
        path: '/shared/file',
        read: { role: 'Member', visibility: 0 },
        write: { role: 'Admin' },
      },
    ],
  );
  assert.equal(
    created.some(({ path }) => path === '/shared/folder'),
    false,
    'folders are not listed as standalone items in the catalog',
  );
  await assert.rejects(
    bridge.createTextItem({
      storeId: 'team:eng',
      path: '/missing-roles',
      value: 'x',
    }),
    (error: unknown) => normalizeCommandError(error).code === 'invalid-request',
  );
  await assert.rejects(
    bridge.createTextItem({
      storeId: 'team:eng',
      path: '/out-of-range-role',
      value: 'x',
      readRole: 'Member:-32769',
      writeRole: 'Admin',
    }),
    (error: unknown) => normalizeCommandError(error).code === 'invalid-request',
  );
  await assert.rejects(
    bridge.createTextItem({
      storeId: 'team:eng',
      path: '/bad-role',
      value: 'x',
      readRole: 'Member: 0',
      writeRole: 'Admin',
    }),
    (error: unknown) => normalizeCommandError(error).code === 'invalid-request',
  );
});

test('account item creation omits roles and item updates preserve existing group roles', async () => {
  const bridge = mockBridge(FIXTURE);
  await bridge.createTextItem({
    storeId: 'acct:personal',
    path: '/account-default',
    value: 'x',
  });
  const account = (await bridge.listCatalog()).items.find(
    (item) => item.path === '/account-default',
  );
  assert.deepEqual(account?.read, { role: 'Owner' });
  assert.deepEqual(account?.write, { role: 'Owner' });
  await assert.rejects(
    bridge.createTextItem({
      storeId: 'acct:personal',
      path: '/account-with-role',
      value: 'x',
      readRole: 'Owner',
      writeRole: 'Owner',
    }),
    (error: unknown) => normalizeCommandError(error).code === 'invalid-request',
  );

  const text = FIXTURE.items.find(
    (item) =>
      item.store === 'team:eng' && item.path === '/deploy/staging-token',
  );
  const file = FIXTURE.items.find(
    (item) => item.store === 'team:eng' && item.path === '/release/bundle.tar',
  );
  assert.ok(text);
  assert.ok(file);
  await bridge.editTextItem({
    storeId: text.store,
    path: text.path,
    version: text.version,
    value: 'changed',
  });
  await bridge.replaceDroppedFile({
    storeId: file.store,
    path: file.path,
    version: file.version,
    sourcePath: '/tmp/replacement',
  });
  const after = (await bridge.listCatalog()).items;
  const edited = after.find(
    (item) => item.store === text.store && item.path === text.path,
  );
  const replaced = after.find(
    (item) => item.store === file.store && item.path === file.path,
  );
  assert.deepEqual(edited?.read, roleDto(text.read));
  assert.deepEqual(edited?.write, roleDto(text.write));
  assert.deepEqual(replaced?.read, roleDto(file.read));
  assert.deepEqual(replaced?.write, roleDto(file.write));
});

test('mock createGroup assigns prefixed team IDs based on group type', async () => {
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

test('mock editTextItem increments item version and returns updated value on read', async () => {
  const bridge = mockBridge(FIXTURE);
  const original = FIXTURE.items.find(
    (item) =>
      item.store === 'acct:personal' && item.path === '/logins/github.com',
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
    (
      await bridge.readItem({
        storeId: edited.store,
        path: edited.path,
        version: edited.version,
      })
    ).value,
    'user: marcus\npassword: replacement\nurl: https://github.com',
  );
});

test('mock pickAndImportFile persists imported file across catalog refreshes', async () => {
  const bridge = mockBridge(FIXTURE);
  assert.deepEqual(
    await bridge.pickAndImportFile({
      storeId: 'acct:personal',
      path: '/documents/picked.pdf',
    }),
    { applied: true },
  );
  const picked = (await bridge.listCatalog()).items.find(
    (item) => item.path === '/documents/picked.pdf',
  );
  assert.ok(picked);
  assert.equal(picked.kind, 'File');
  assert.equal(picked.version, 1);
});

test('mock resumeGroupCreation activates an inactive team', async () => {
  const bridge = mockBridge(FIXTURE);
  await bridge.resumeGroupCreation('team:homelab');
  const store = (await bridge.listCatalog()).stores.find(
    (candidate) => candidate.id === 'team:homelab',
  );
  assert.ok(store && store.kind === 'team');
  assert.equal(store.active, true);
});

test('mock removeFederatedGroup requires both host ID and team ID to match', async () => {
  const active = { ...FIXTURE.federation[0], active: true };
  const bridge = mockBridge({ ...FIXTURE, federation: [active] });
  await assert.rejects(
    bridge.removeFederatedGroup({
      storeId: active.store,
      remoteHostIdHex: `02${'ff'.repeat(32)}`,
      remoteTeamIdHex: active.remote_team_id_hex,
    }),
    (error: unknown) => normalizeCommandError(error).code === 'invalid-request',
  );
  assert.equal((await bridge.listFederation(active.store)).length, 1);
  assert.deepEqual(
    await bridge.removeFederatedGroup({
      storeId: active.store,
      remoteHostIdHex: active.remote_host_id_hex,
      remoteTeamIdHex: active.remote_team_id_hex,
    }),
    { applied: true },
  );
  assert.equal((await bridge.listFederation(active.store)).length, 0);
});

test('the mock rejects member and sharing updates for an active ad-hoc group', async () => {
  const snapshot = {
    ...FIXTURE,
    stores: FIXTURE.stores.map((store) =>
      store.id === 'team:homelab' && store.kind === 'team'
        ? { ...store, active: true as const }
        : store,
    ),
  };
  const bridge = mockBridge(snapshot);
  await assert.rejects(
    bridge.addGroupMember({
      storeId: 'team:homelab',
      username: 'person',
      destination: { role: 'Member', visibility: 0 },
    }),
    (error: unknown) =>
      normalizeCommandError(error).code === 'group-management-unavailable',
  );
  await assert.rejects(
    bridge.admitGroup({
      storeId: 'team:homelab',
      remoteStoreId: 'team:eng',
      visibility: 0,
    }),
    (error: unknown) =>
      normalizeCommandError(error).code === 'group-management-unavailable',
  );
});

test('enqueueProfileWork serializes tasks for the same profile', async () => {
  const bridge = mockBridge(FIXTURE);
  const seen: string[] = [];
  let release!: () => void;
  const hold = new Promise<void>((resolve) => {
    release = resolve;
  });
  const first = enqueueProfileWork(bridge, 'personal', async () => {
    seen.push('start-a');
    await hold;
    seen.push('end-a');
    return 1;
  });
  const second = enqueueProfileWork(bridge, 'personal', async () => {
    seen.push('b');
    return 2;
  });
  release();
  assert.deepEqual(await Promise.all([first, second]), [1, 2]);
  assert.deepEqual(seen, ['start-a', 'end-a', 'b']);
});

test('native loadSnapshot serializes group detail requests per profile', async () => {
  const base = mockBridge(FIXTURE);
  let legacyCalls = 0;
  const active = new Map<string, number>();
  const peak = new Map<string, number>();
  const enter = (profile: string): void => {
    const next = (active.get(profile) ?? 0) + 1;
    active.set(profile, next);
    peak.set(profile, Math.max(peak.get(profile) ?? 0, next));
  };
  const leave = (profile: string): void => {
    active.set(profile, (active.get(profile) ?? 1) - 1);
  };
  const around = async <T>(
    profile: string,
    work: () => Promise<T>,
  ): Promise<T> => {
    enter(profile);
    try {
      await new Promise((resolve) => setTimeout(resolve, 5));
      return await work();
    } finally {
      leave(profile);
    }
  };
  const bridge: Bridge = {
    ...base,
    native: true,
    listGroupDetails: async (storeId) => {
      const store = FIXTURE.stores.find((entry) => entry.id === storeId);
      assert.ok(store);
      return around(store.server, () => base.listGroupDetails(storeId));
    },
    listParties: async (storeId) => {
      legacyCalls += 1;
      return base.listParties(storeId);
    },
    listFederation: async (storeId) => {
      legacyCalls += 1;
      return base.listFederation(storeId);
    },
  };
  await loadSnapshot(bridge);
  assert.ok((peak.get('personal') ?? 0) <= 1);
  assert.ok((peak.get('acme') ?? 0) <= 1);
  assert.equal(peak.get('personal'), 1);
  assert.equal(legacyCalls, 0);
});

test('maintenance snapshots decode typed operation and restoration outcomes', () => {
  const snapshot = decodeMaintenanceSnapshot({
    state: 'complete',
    generation: 12,
    revision: 31,
    kind: 'export',
    operation: {
      status: 'completed',
    },
    disposition: {
      status: 'restoration-failed',
      root: '/tmp/current',
      error: {
        code: 'agent-start-failed',
        message: 'restart failed',
        retryable: true,
        ambiguous: false,
        fatal: false,
      },
    },
  });
  assert.equal(snapshot.state, 'complete');
  if (snapshot.state !== 'complete') return;
  assert.deepEqual(snapshot.operation, { status: 'completed' });
  assert.equal(snapshot.disposition.status, 'restoration-failed');
});

test('maintenance snapshot decoder rejects impossible loose values', () => {
  assert.throws(() =>
    decodeMaintenanceSnapshot({
      state: 'active',
      generation: 1,
      revision: 1,
      kind: 'export',
      phase: 'complete',
    }),
  );
  assert.throws(() =>
    decodeMaintenanceSnapshot({
      state: 'complete',
      generation: 1,
      revision: 1,
      kind: 'import',
      operation: { status: 'failed', error: 'message only' },
      disposition: { status: 'continue-current-root' },
    }),
  );
});
