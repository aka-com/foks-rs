import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeAccounts } from '../src/bridge/accounts-groups';
import { vaultCommands } from '../src/bridge/commands-vault';
import { createBridge } from '../src/bridge/create';
import {
  decodeServerStatus,
  type ServerStatusSnapshot,
} from '../src/bridge/servers';
import { createTransport, type Transport } from '../src/bridge/transport';
import { FIXTURE } from '../src/mock/fixture';
import {
  MOCK_HOST_MEMBERS,
  mockBridge,
  mockHostCommands,
} from '../src/mock/mock-bridge';
import { mockCommand, mockHost } from '../src/mock/host';
import { mockState } from '../src/mock/state';
import { MOCK_YUBI_COMMANDS } from '../src/mock/yubikey';
import { accountWire, serverStatusWire } from '../src/mock/wire';
import type { YubiCommand } from '../src/bridge/yubikey';
import type { Account } from '../src/model/types';
import type { SsoAction } from '../src/sso-contract';

/** A wire value as it crosses the IPC boundary. */
const overWire = (value: unknown): unknown =>
  JSON.parse(JSON.stringify(value)) as unknown;

test('the mock host answers through the bridge decoders and rejects what they reject', async () => {
  let response: unknown = {
    store: 'acct:personal',
    path: '/a',
    version: 1,
    value: 'text',
    extra: undefined,
  };
  const bridge = createBridge(
    createTransport(
      mockHost({
        read_item: mockCommand(() => response),
        create_folder: () => ({ applied: 'yes' }),
      }),
    ),
  );
  const request = {
    storeId: 'acct:personal',
    path: '/a',
    version: 1,
    direntId: 'aa'.repeat(16),
  };
  // Serialized as Tauri does, so an undefined field is not part of the reply.
  assert.deepEqual(await bridge.readItem(request), {
    store: 'acct:personal',
    path: '/a',
    version: 1,
    value: 'text',
  });
  response = { ...(response as object), version: '1' };
  await assert.rejects(bridge.readItem(request), {
    code: 'invalid-response',
    origin: 'response',
    ambiguous: false,
  });
  await assert.rejects(
    bridge.createFolder({ storeId: 'acct:personal', path: '/b' }),
    { code: 'invalid-response', origin: 'response', ambiguous: true },
  );
  await assert.rejects(bridge.storeUsage('acct:personal'), {
    code: 'invalid-request',
    origin: 'invoke',
    message: 'The mock host has no command store_usage.',
  });
});

test('a mock handler fault keeps its message through the transport', async () => {
  const bridge = createBridge(
    createTransport(
      mockHost({
        store_usage: () => {
          throw new Error('The handler failed.');
        },
      }),
    ),
  );
  await assert.rejects(bridge.storeUsage('acct:personal'), {
    origin: 'invoke',
    message: 'The handler failed.',
  });
});

test('the mock wire encoders invert the bridge decoders', () => {
  const accounts: Account[] = [
    {
      store: 'acct:personal',
      server: 'foks.app',
      alias: 'alice',
      username: 'alice',
    },
    {
      store: 'acct:work',
      server: 'work.example',
      localAlias: 'work',
      alias: 'alice@work',
      username: 'alice',
    },
  ];
  assert.deepEqual(
    decodeAccounts(overWire(accounts.map((account) => accountWire(account)))),
    accounts,
  );
  const host = {
    lookupName: 'foks.app',
    canonicalName: 'foks.app',
    hostId: '02' + 'ab'.repeat(32),
    chain: 3,
    epoch: 1,
  };
  const statuses: ServerStatusSnapshot[] = [
    {
      profile: 'foks.app',
      configuredEndpoint: 'foks.app:443',
      host,
      leaseRequired: false,
      leaseExpiresAt: null,
      compatibility: { status: 'not-required' },
      chatSupported: true,
    },
    {
      profile: 'foks.app',
      configuredEndpoint: 'foks.app:443',
      host: null,
      leaseRequired: true,
      leaseExpiresAt: null,
      compatibility: { status: 'required-unavailable' },
      chatSupported: null,
    },
    {
      profile: 'foks.app',
      configuredEndpoint: 'foks.app:443',
      host,
      leaseRequired: true,
      leaseExpiresAt: 1_900_000_000,
      compatibility: {
        status: 'required',
        expiresAt: 1_900_000_000,
        capabilities: ['signup', 'kv'],
      },
      chatSupported: false,
    },
    {
      profile: 'foks.app',
      configuredEndpoint: 'foks.app:443',
      host,
      leaseRequired: true,
      leaseExpiresAt: 1_900_000_000,
      compatibility: {
        status: 'incompatible',
        expiresAt: 1_900_000_000,
        reason: 'protocol-mismatch',
      },
      chatSupported: true,
    },
  ];
  for (const status of statuses)
    assert.deepEqual(
      decodeServerStatus(overWire(serverStatusWire(status))),
      status,
    );
});

test('mock vault failures carry the host codes and the transport normalization', async () => {
  const bridge = mockBridge(FIXTURE);
  const item = (await bridge.listCatalog()).items.find(
    (entry) => entry.path === '/logins/github.com',
  );
  assert.ok(item);
  await assert.rejects(
    bridge.readItem({
      storeId: item.store,
      path: item.path,
      version: item.version + 1,
      direntId: item.direntId,
    }),
    {
      code: 'item-version-mismatch',
      origin: 'invoke',
      retryable: true,
      ambiguous: false,
    },
  );
  await assert.rejects(
    bridge.readItem({
      storeId: item.store,
      path: '/missing',
      version: 1,
      direntId: item.direntId,
    }),
    { code: 'item-not-found', origin: 'invoke', ambiguous: false },
  );
  // Another dirent at the same path and version: the listed entry was
  // replaced, and the read refuses it.
  await assert.rejects(
    bridge.readItem({
      storeId: item.store,
      path: item.path,
      version: item.version,
      direntId: 'ff'.repeat(16),
    }),
    {
      code: 'item-replaced',
      message: 'This item was replaced. Refresh the vault before reading it.',
      origin: 'invoke',
      retryable: true,
      ambiguous: false,
    },
  );
  await assert.rejects(
    bridge.editTextItem({
      storeId: item.store,
      path: item.path,
      version: item.version,
      direntId: 'stale',
      value: 'changed',
    }),
    { code: 'conflict', origin: 'invoke', ambiguous: false },
  );
});

test('mock password import progress arrives through the transport channel', async () => {
  const bridge = mockBridge(FIXTURE);
  const preview = await bridge.beginPasswordImport();
  assert.ok(preview);
  const progress: string[] = [];
  const outcomes = await bridge.commitPasswordImport(
    {
      importId: preview.id,
      entries: [0, 1],
      storeId: 'acct:personal',
      folder: '/imported',
    },
    (outcome) => progress.push(`${outcome.index}:${outcome.status}`),
  );
  assert.deepEqual(progress, ['0:created', '1:created']);
  assert.deepEqual(
    outcomes.map((outcome) => outcome.status),
    ['created', 'created'],
  );
});

test('the mock host answers every command its members send, and no other', async () => {
  // A transport that records the command each member sends, and no more.
  const sent = new Set<string>();
  const record = async (command: string): Promise<never> => {
    sent.add(command);
    return undefined as never;
  };
  const recording: Transport = {
    invoke: async (command) => {
      sent.add(command);
    },
    checked: record,
    checkedMutation: record,
    listen: async () => () => undefined,
    channel: () => ({ onmessage: () => undefined }),
  };
  const recorder = createBridge(recording) as unknown as Record<
    string,
    (...args: unknown[]) => Promise<unknown>
  >;
  const members: string[] = [
    ...Object.keys(vaultCommands(recording)),
    ...MOCK_HOST_MEMBERS,
  ];
  // A request whose every field is an empty list, so a member that maps a
  // list in its request still sends its command.
  const request: unknown = new Proxy({}, { get: () => [] });
  for (const member of members) {
    const method = recorder[member];
    // runYubi sends the command its request names.
    const calls =
      member === 'runYubi'
        ? MOCK_YUBI_COMMANDS.map((command) => [{ command, args: {} }])
        : // Without arguments too, so a member whose command depends on an
          // optional argument, such as listCatalog's progress listener,
          // sends each of its commands.
          [[], [request, request, request, request]];
    for (const args of calls)
      try {
        await method(...args);
      } catch {
        // A member that reads a field of a missing request sends nothing.
      }
  }
  assert.deepEqual(
    [...sent].sort(),
    Object.keys(mockHostCommands(mockState(FIXTURE))).sort(),
  );
});

test('mock team and server failures carry the host codes and the transport normalization', async () => {
  const bridge = mockBridge(FIXTURE);
  await assert.rejects(
    bridge.addGroupMember({
      storeId: 'team:eng',
      username: 'sam.ortiz',
      destination: { role: 'Member', visibility: 0 },
    }),
    {
      code: 'conflict',
      origin: 'invoke',
      retryable: false,
      ambiguous: false,
    },
  );
  await assert.rejects(
    bridge.promoteGroupMember({
      storeId: 'team:homelab',
      username: 'satoshi',
      destination: { role: 'Admin' },
    }),
    { code: 'inactive-group', origin: 'invoke', ambiguous: false },
  );
  await assert.rejects(bridge.setServerLabel('missing', 'Home'), {
    code: 'profile-not-found',
    origin: 'invoke',
    ambiguous: false,
  });
  assert.deepEqual(await bridge.setServerLabel('personal', ' Home '), {
    profile: 'personal',
    label: 'Home',
    changed: true,
  });
  assert.deepEqual(
    await bridge.removeGroupMember({
      storeId: 'team:eng',
      username: 'sam.ortiz',
    }),
    { applied: true },
  );
  assert.equal(
    (await bridge.listParties('team:eng')).some(
      (party) => party.username === 'sam.ortiz',
    ),
    false,
  );
});

test('mock enrollment answers pass the decoders and failures carry the host codes', async () => {
  const bridge = mockBridge(FIXTURE);
  // The fixture's key on a card has a hardware key id, which the decoder
  // accepts only with 66 hex digits after its `08`.
  assert.ok(
    (await bridge.listAccountDevices('acct:personal')).some(
      (device) =>
        device.name === 'Pocket YubiKey' && /^08[0-9a-f]{66}$/.test(device.id),
    ),
  );
  await assert.rejects(bridge.resumeDevicePairingOffer('acct:personal'), {
    code: 'pending-operation-not-found',
    origin: 'invoke',
    ambiguous: false,
  });
  const offer = await bridge.startDevicePairing('acct:personal');
  assert.deepEqual(
    await bridge.resumeDevicePairingOffer('acct:personal'),
    offer,
  );
  await assert.rejects(bridge.resetServer('personal', 'other', 'token'), {
    code: 'invalid-request',
    origin: 'invoke',
    ambiguous: false,
  });
});

test('every mock YubiKey and account host response passes the bridge decoders', async () => {
  const bridge = mockBridge(FIXTURE);
  const key = { profile: 'personal', alias: 'primary key' };
  const card = {
    deviceName: 'Pocket YubiKey',
    cardSerial: 20993145,
    signingSlot: 0x82,
    pqSlot: 0x83,
    pin: '123456',
    puk: '12345678',
    pinAttempts: 3,
    pukAttempts: 3,
  };
  const passphrase = {
    ...key,
    pin: '123456',
    passphrase: 'correct horse',
    confirmation: 'correct horse',
  };
  const requests: YubiCommand[] = [
    {
      command: 'create_yubi_account',
      args: {
        ...card,
        profile: 'personal',
        alias: 'spare key',
        username: 'satoshi',
        email: 'satoshi@example.com',
        invite: '',
      },
    },
    { command: 'resume_yubi_account', args: { ...key, pin: '123456' } },
    {
      command: 'provision_yubi_device',
      args: { ...card, accountStoreId: 'acct:personal', targetAlias: 'laptop' },
    },
    {
      command: 'sync_yubi_account',
      args: { ...key, pin: '123456', withFederation: true },
    },
    { command: 'yubi_pin_status', args: key },
    {
      command: 'change_yubi_pin',
      args: { ...key, oldPin: '123456', newPin: '654321' },
    },
    { command: 'set_yubi_passphrase', args: passphrase },
    { command: 'change_yubi_passphrase', args: passphrase },
    {
      command: 'verify_yubi_passphrase',
      args: { ...key, pin: '123456', passphrase: 'correct horse' },
    },
    {
      command: 'change_yubi_puk',
      args: { ...key, oldPuk: '12345678', newPuk: '87654321' },
    },
    {
      command: 'unblock_yubi_pin',
      args: { ...key, puk: '12345678', newPin: '654321' },
    },
    { command: 'rotate_yubi_management_key', args: { ...key, pin: '123456' } },
    { command: 'resume_yubi_management_key', args: key },
    {
      command: 'recover_yubi_management_key',
      args: { accountStoreId: 'acct:personal', yubiAlias: 'primary key' },
    },
    { command: 'recover_yubi_subkey', args: { ...key, pin: '123456' } },
    {
      command: 'revoke_yubi_device',
      args: {
        accountStoreId: 'acct:personal',
        yubiAlias: 'primary key',
        confirmation: 'primary key',
      },
    },
  ];
  assert.deepEqual(
    requests.map((request) => request.command).sort(),
    [...MOCK_YUBI_COMMANDS].sort(),
  );
  for (const request of requests)
    await assert.doesNotReject(bridge.runYubi(request), request.command);
  assert.deepEqual(await bridge.listYubiCards('personal'), [
    { serial: 20993145 },
  ]);
  assert.ok(
    (await bridge.listYubiAccounts('personal')).some(
      (entry) => entry.alias === 'spare key',
    ),
  );

  assert.deepEqual(await bridge.configureWebAdmin('acme', 'work', 'browser'), {
    ok: true,
  });
  assert.deepEqual(
    await bridge.openSsoBrowser('acme', 'work', '1'.repeat(32)),
    { ok: true },
  );
  assert.deepEqual(
    await bridge.botAccount('acme', 'work', { action: 'list' }),
    { rows: [], message: 'Bot credentials require a connected agent.' },
  );
  const ssoActions: SsoAction[] = [
    { action: 'account-status', pin: null },
    { action: 'begin', purpose: 'link-existing', pin: null },
    { action: 'poll', operation_id: '1'.repeat(32) },
  ];
  for (const action of ssoActions)
    await assert.doesNotReject(
      bridge.sso('acme', 'work', action),
      action.action,
    );
  assert.deepEqual(await bridge.discoverGroups('personal', 'personal'), {
    accountAlias: 'personal',
    groups: [],
  });
  const invited = bridge.firstRunFixture?.invited;
  assert.ok(invited);
  assert.deepEqual(
    (
      await bridge.discoverGroups(invited.profile, invited.accountAlias)
    ).groups.map((group) => group.alias),
    ['engineering'],
  );
});
