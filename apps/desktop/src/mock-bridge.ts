/** A deterministic command bridge for render and browser acceptance tests. */

import { notificationKey } from './chat/local-contract';
import { mockInvitations } from './invitation-mock';
import { mockChat } from './chat-mock';
import type {
  Bridge,
  CatalogDto,
  DownloadResponse,
  ItemRequest,
  KvRoleInput,
  ReadItemResponse,
} from './bridge';
import { roleDto } from './bridge';
import { FIXTURE } from './fixture';
import { catalog } from './model/lease';
import { parseRole, roleRank, visibilityOf } from './model/roles';
import { itemKey } from './model/types';
import type { RoleWire, Store, AgentSnapshot } from './model/types';
import {
  decodeFirstRunCheckpoint,
  FIRST_RUN_CHECKPOINT_KEY,
} from './first-run-state';

export class VersionMismatchError extends Error {
  constructor(
    readonly path: string,
    readonly asked: number,
    readonly found: number,
  ) {
    super(
      `${path} was updated to version ${found} (requested ${asked}). Refresh to view changes.`,
    );
    this.name = 'VersionMismatchError';
  }
}

export type AppLifecycleRequest = 'restart' | 'quit';

export const mockAppLifecycleRequests: AppLifecycleRequest[] = [];

export function mockBridge(snapshot: AgentSnapshot = FIXTURE): Bridge {
  const ssoModes = new Map<string, import('./sso-contract').SsoPurpose>();
  const stores: Store[] = snapshot.stores.map((store) => ({ ...store }));
  const servers = snapshot.servers.map((server) => ({ ...server }));
  const serverProbes = new Map(
    snapshot.servers.map((server) => [server.id, server.configuredProbe]),
  );
  const accounts = snapshot.accounts.map((account) => ({ ...account }));
  let items = snapshot.items.map((item) => ({ ...item }));
  let parties = snapshot.parties.map((party) => ({ ...party }));
  let federation = snapshot.federation.map((entry) => ({ ...entry }));
  const contents = new Map(
    items.map((item) => [
      itemKey(item),
      item.kind === 'Link'
        ? (item.target ?? '')
        : (snapshot.plaintext[itemKey(item)] ?? item.value ?? ''),
    ]),
  );
  const select = (request: ItemRequest) => {
    const item = items.find(
      (candidate) =>
        candidate.store === request.storeId && candidate.path === request.path,
    );
    if (!item)
      throw new Error(
        `Item '${request.path}' was not found in vault '${request.storeId}'.`,
      );
    if (item.version !== request.version) {
      throw new VersionMismatchError(item.path, request.version, item.version);
    }
    return item;
  };
  const hoverListeners = new Set<(event: { hovering: boolean }) => void>();
  const pathListeners = new Set<(paths: string[]) => void>();
  const failure = (code: string, message: string) => ({
    code,
    message,
    retryable: code === 'agent-lost',
    ambiguous: false,
    fatal: code === 'agent-lost',
    details: { kind: code },
  });
  const decodeCreateRole = (role: KvRoleInput): RoleWire => {
    if (role === 'Owner' || role === 'Admin') return { role };
    const match = /^Member:(0|-?[1-9]\d*)$/.exec(role);
    const visibility = Number(match?.[1]);
    if (
      !match ||
      !Number.isInteger(visibility) ||
      visibility < -32768 ||
      visibility > 32767
    ) {
      throw failure('invalid-request', 'Choose a valid group item role.');
    }
    return { role: 'Member', visibility };
  };
  const createRoles = (
    storeId: string,
    readRole?: KvRoleInput,
    writeRole?: KvRoleInput,
  ): { read: RoleWire; write: RoleWire } => {
    const store = stores.find((candidate) => candidate.id === storeId);
    if (!store) throw failure('store-not-found', 'This store was not found.');
    if (store.kind === 'account') {
      if (readRole !== undefined || writeRole !== undefined)
        throw failure(
          'invalid-request',
          'Account items always use the Owner role.',
        );
      return { read: { role: 'Owner' }, write: { role: 'Owner' } };
    }
    if (!store.active)
      throw failure(
        'inactive-group',
        'Finish setting up this group before making changes.',
      );
    if (readRole === undefined || writeRole === undefined)
      throw failure(
        'invalid-request',
        'Specify both read and write roles for the group item.',
      );
    return {
      read: decodeCreateRole(readRole),
      write: decodeCreateRole(writeRole),
    };
  };
  const fixtureEngineeringStore = snapshot.stores.find(
    (store) => store.id === 'team:eng',
  );
  const fixtureEngineering =
    fixtureEngineeringStore?.kind === 'team'
      ? fixtureEngineeringStore
      : undefined;
  const fixtureHouseholdStore = snapshot.stores.find(
    (store) => store.id === 'team:household',
  );
  const fixtureHousehold =
    fixtureHouseholdStore?.kind === 'team' ? fixtureHouseholdStore : undefined;
  const firstRunFixture = {
    invited: {
      profile: 'acme',
      accountAlias: 'sol',
      server: 'foks.acme-corp.com',
      typo: 'foks.acme-corp.co',
      username: 'sol',
      deviceName: "Sol's MacBook Air",
      admin: 'sam.ortiz',
      groupName: 'Engineering',
      groupAlias: fixtureEngineering?.alias,
      groupTeamIdHex: fixtureEngineering?.team_id_hex,
      report: {
        profile: 'acme',
        acceptance: 'inserted' as const,
        lookupName: 'foks.acme-corp.com',
        canonicalName: 'foks.acme-corp.com',
        hostId:
          '024d17e390a5c2f8e16d7b39c04a8e5f2d1c6b7a9038e4f5d6c1a2b3e7f0948d1c',
        chain: 33,
        epoch: 90417,
      },
    },
    own: {
      profile: 'personal',
      accountAlias: 'personal',
      server: 'foks.example.net',
      typo: 'foks.example.ne',
      username: 'satoshi',
      deviceName: 'MacBook Pro',
      groupName: 'Household',
      groupAlias: fixtureHousehold?.alias,
      groupTeamIdHex: fixtureHousehold?.team_id_hex,
      report: {
        profile: 'personal',
        acceptance: 'inserted' as const,
        lookupName: 'foks.example.net',
        canonicalName: 'foks.example.net',
        hostId:
          '0231c2aa07e4b1d86c3f52a09e7d41c8b6f0a2d3e95c17b48f6a0d2e3c5b719a4f',
        chain: 12,
        epoch: 4821,
      },
    },
    backupPhrase:
      'orbit velvet lantern cactus mirror harbor pistol thumb copper fossil meadow rotate silent wagon bright ladder ivory',
  };
  let appLocked = false;
  const notificationSettings = {
    enabled: false,
    previews: false,
    overrides: {} as Record<string, boolean>,
  };
  const appLockState = () => ({
    locked: appLocked,
    available: true,
    mechanism: 'password' as const,
  });
  const pending = new Map<
    string,
    {
      kind: 'account-signup' | 'account-recovery';
      alias: string;
      target?: string;
    }[]
  >();
  const serverHosts = new Map<
    string,
    {
      lookupName: string;
      canonicalName: string;
      hostId: string;
      chain: number;
      epoch: number;
    }
  >();
  const hostIds: Record<string, string> = {
    personal: `02${'9f31c2aa'.repeat(8)}`,
    acme: `02${'b04d17e3'.repeat(8)}`,
    partner: `02${'04c8b19e'.repeat(8)}`,
  };
  for (const server of servers) {
    if (server.host_id && server.chain !== null && server.epoch !== null) {
      serverHosts.set(server.id, {
        lookupName: server.configuredProbe,
        canonicalName: server.configuredProbe,
        hostId: hostIds[server.id] ?? `02${'1'.repeat(64)}`,
        chain: server.chain,
        epoch: server.epoch,
      });
    }
  }
  const pairingOffers = new Map<string, string>();
  const pairingAcceptances = new Set<string>();
  const resetTokens = new Set<string>();
  const deviceRows = new Map<
    string,
    {
      id: string;
      name?: string;
      role: 'owner' | 'admin' | 'member';
      current: boolean;
    }[]
  >();
  const backupRows = new Map<
    string,
    { backupAlias: string; accountAlias: string; backupId: string }[]
  >();
  // `list_yubi_accounts` returns results for a single profile. The mock tracks
  // the associated server and enrollment status per account, filtering
  // enrollments by account and marking incomplete enrollments as pending.
  const yubi: {
    alias: string;
    server: string;
    state: 'pending' | 'complete';
    serial?: number;
  }[] = snapshot.yubiAccounts.map((entry) => ({
    alias: entry.alias,
    server: entry.server,
    state: entry.state,
    serial: entry.serial,
  }));
  const accountStore = (id: string) =>
    stores.find((store) => store.id === id && store.kind === 'account');
  for (const store of stores) {
    if (store.kind !== 'account') continue;
    const rows =
      store.account === 'personal'
        ? [
            ...snapshot.devices.map((device) => ({
              id: device.id_hex.replace(/^02/, '04'),
              name: device.name,
              role: 'owner' as const,
              current: device.current,
            })),
            // Hardware security key device record (prefix `08`), providing
            // fixture data that distinguishes card-bound keys from host keys.
            {
              id: `08${'5'.repeat(64)}`,
              name: 'Pocket YubiKey',
              role: 'member' as const,
              current: false,
            },
          ]
        : [
            {
              id: `04${'8'.repeat(64)}`,
              name: 'MacBook Pro',
              role: 'owner' as const,
              current: true,
            },
          ];
    deviceRows.set(store.id, rows);
    backupRows.set(
      store.id,
      store.account === 'personal'
        ? [
            {
              backupAlias: 'paper-backup',
              accountAlias: store.account,
              backupId: `10${'4'.repeat(64)}`,
            },
          ]
        : [],
    );
  }
  const restoreFirstRunAccount = (): void => {
    if (typeof window === 'undefined') return;
    const saved = decodeFirstRunCheckpoint(
      window.localStorage.getItem(FIRST_RUN_CHECKPOINT_KEY),
    );
    if (!saved?.profile) return;
    // The preview simulates persisted native state after a successful mutation.
    // Keep its host binding and profile IDs consistent with the real DTOs.
    const server = servers.find((entry) => entry.id === saved.profile?.profile);
    if (server) server.host_id = saved.profile.hostId;
    const account =
      saved.account ??
      (saved.provisionedAccount
        ? {
            alias: saved.provisionedAccount.alias,
            username: firstRunFixture[saved.path].username,
          }
        : undefined);
    if (!account) return;
    const id = `acct:${account.alias}`;
    if (!stores.some((store) => store.id === id)) {
      stores.push({
        id,
        kind: 'account',
        name: 'Personal',
        server: saved.profile.profile,
        account: account.alias,
      });
    }
    if (!accounts.some((account) => account.store === id)) {
      accounts.push({
        store: id,
        alias: account.alias,
        username: account.username,
        server: saved.profile.profile,
      });
    }
  };
  const ensureFirstRunAccount = (
    profile: string,
    alias: string,
    username: string,
  ): void => {
    const id = `acct:${alias}`;
    if (!stores.some((store) => store.id === id)) {
      stores.push({
        id,
        kind: 'account',
        name: 'Personal',
        server: profile,
        account: alias,
      });
    }
    const existing = accounts.find((account) => account.store === id);
    if (existing) {
      existing.store = id;
      existing.username = username;
    } else accounts.push({ store: id, alias, username, server: profile });
  };
  const assertFree = (storeId: string, path: string): void => {
    if (items.some((item) => item.store === storeId && item.path === path)) {
      throw failure('already-exists', `${path} already exists.`);
    }
  };
  const assertNamedGroup = (storeId: string): void => {
    const store = stores.find((candidate) => candidate.id === storeId);
    if (!store || store.kind !== 'team')
      throw failure('store-not-found', 'The group was not found.');
    if (!store.active)
      throw failure('inactive-group', 'The group setup is incomplete.');
    if (store.team_kind !== 'named')
      throw failure(
        'group-management-unavailable',
        'Managing members and shared access requires a named group.',
      );
  };
  const catalogResponse = (): CatalogDto => ({
    profiles: [...new Set(servers.map((server) => server.id))],
    stores: stores.map((store) => ({ ...store })),
    knownStores: stores.map((store) => ({ ...store })),
    inventory: [...new Set(servers.map((server) => server.id))].map(
      (profile) => ({
        profile,
        accountsComplete: true,
        teamsComplete: true,
      }),
    ),
    items: catalog({ ...snapshot, items }).map((item) => ({
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
  });
  const chat = mockChat(snapshot);
  return {
    native: false,
    maintainClientState: async () => {
      throw new Error('State maintenance requires the native application.');
    },
    clientStateMaintenanceStatus: async () => ({
      state: 'idle',
      generation: 0,
      revision: 0,
    }),
    relocateClientState: async () => {
      throw new Error('State maintenance requires the native application.');
    },
    chat,
    cancelChat: chat.cancel,
    fixtureSnapshot: snapshot,
    firstRunFixture,
    appLockState: async () => appLockState(),
    windowState: async () => ({ maximized: false, fullscreen: false }),
    setTrafficLightsVisible: async () => {},
    lockApp: async () => {
      appLocked = true;
      return appLockState();
    },
    unlockApp: async () => {
      appLocked = false;
      return appLockState();
    },
    restartApp: async () => {
      mockAppLifecycleRequests.push('restart');
    },
    quitApp: async () => {
      mockAppLifecycleRequests.push('quit');
    },
    agentStatus: () => Promise.resolve({ ...snapshot.agent }),
    appInfo: async () => ({
      version: '0.3.0',
      agentSocket: '/private/foks/agent.sock',
    }),
    listCatalog: () => {
      restoreFirstRunAccount();
      return Promise.resolve(catalogResponse());
    },
    listStores: () => Promise.resolve({ ...catalogResponse(), items: [] }),
    listServers: () =>
      Promise.resolve(servers.map((server) => ({ ...server }))),
    listAccounts: () => {
      restoreFirstRunAccount();
      // Fixture and restored accounts already map directly to their store IDs.
      return Promise.resolve(accounts.map((account) => ({ ...account })));
    },
    listGroupDetails: (storeId) =>
      Promise.resolve({
        parties: {
          status: 'success' as const,
          value: parties
            .filter((party) => party.store === storeId)
            .map((party) => ({ ...party })),
        },
        federation: {
          status: 'success' as const,
          value: federation
            .filter((entry) => entry.store === storeId)
            .map((entry) => ({ ...entry })),
        },
      }),
    listParties: (storeId) =>
      Promise.resolve(
        parties
          .filter((party) => party.store === storeId)
          .map((party) => ({ ...party })),
      ),
    listFederation: (storeId) =>
      Promise.resolve(
        federation
          .filter((entry) => entry.store === storeId)
          .map((entry) => ({ ...entry })),
      ),
    readItem: (request: ItemRequest): Promise<ReadItemResponse> => {
      const item = select(request);
      return Promise.resolve({
        store: item.store,
        path: item.path,
        version: item.version,
        value: contents.get(itemKey(item)) ?? '',
      });
    },
    copyItemValue: (request) => {
      select(request);
      return Promise.resolve({ ok: true });
    },
    copyItemPath: (request) => {
      select(request);
      return Promise.resolve({ ok: true });
    },
    downloadFile: (request): Promise<DownloadResponse> => {
      select(request);
      return Promise.resolve({ saved: true });
    },
    createTextItem: async ({ storeId, path, value, readRole, writeRole }) => {
      assertFree(storeId, path);
      const roles = createRoles(storeId, readRole, writeRole);
      items.push({
        store: storeId,
        path,
        kind: 'Secret',
        size: value.length,
        version: 1,
        ...roles,
        value,
      });
      contents.set(`${storeId}|${path}`, value);
      return { applied: true };
    },
    createLink: async ({ storeId, path, target, readRole, writeRole }) => {
      assertFree(storeId, path);
      const roles = createRoles(storeId, readRole, writeRole);
      items.push({
        store: storeId,
        path,
        kind: 'Link',
        size: target.length,
        version: 1,
        ...roles,
        target,
      });
      contents.set(`${storeId}|${path}`, target);
      return { applied: true };
    },
    createFolder: async ({ storeId, path, readRole, writeRole }) => {
      assertFree(storeId, path);
      const roles = createRoles(storeId, readRole, writeRole);
      items.push({
        store: storeId,
        path,
        kind: 'Folder',
        size: 0,
        version: 1,
        ...roles,
      });
      return { applied: true };
    },
    editTextItem: async ({ storeId, path, version, value }) => {
      const index = items.findIndex(
        (item) => item.store === storeId && item.path === path,
      );
      const item = index < 0 ? undefined : items[index];
      if (!item)
        throw failure('conflict', `Item '${path}' was deleted or moved.`);
      if (item.version !== version)
        throw failure('conflict', `${path} changed first.`);
      items[index] = {
        ...item,
        version: version + 1,
        size: value.length,
        value,
      };
      contents.set(`${storeId}|${path}`, value);
      return { applied: true };
    },
    removeItem: async ({ storeId, path, version }) => {
      const item = items.find(
        (candidate) => candidate.store === storeId && candidate.path === path,
      );
      if (!item || item.version !== version)
        throw failure('conflict', `${path} changed first.`);
      items = items.filter((candidate) => candidate !== item);
      contents.delete(`${storeId}|${path}`);
      return { applied: true };
    },
    importDroppedFile: async ({ storeId, path, readRole, writeRole }) => {
      assertFree(storeId, path);
      const roles = createRoles(storeId, readRole, writeRole);
      items.push({
        store: storeId,
        path,
        kind: 'File',
        size: 0,
        version: 1,
        ...roles,
      });
      return { applied: true };
    },
    pickAndImportFile: async ({ storeId, path, readRole, writeRole }) => {
      assertFree(storeId, path);
      const roles = createRoles(storeId, readRole, writeRole);
      items.push({
        store: storeId,
        path,
        kind: 'File',
        size: 0,
        version: 1,
        ...roles,
      });
      return { applied: true };
    },
    replaceDroppedFile: async ({ storeId, path, version }) => {
      const item = items.find(
        (candidate) => candidate.store === storeId && candidate.path === path,
      );
      if (!item || item.version !== version)
        throw failure('conflict', `${path} changed first.`);
      item.version += 1;
      return { applied: true };
    },
    pickAndReplaceFile: async ({ storeId, path, version }) => {
      const item = items.find(
        (candidate) => candidate.store === storeId && candidate.path === path,
      );
      if (!item || item.version !== version)
        throw failure('conflict', `${path} changed first.`);
      item.version += 1;
      return { applied: true };
    },
    resumeGroupCreation: async (storeId) => {
      const store = stores.find((candidate) => candidate.id === storeId);
      if (!store || store.kind !== 'team' || store.active) {
        throw failure(
          'invalid-request',
          'This group has already completed setup.',
        );
      }
      store.active = true;
      return { applied: true };
    },
    takeAgentConnectionLoss: async () => null,
    retryAgentConnection: async () => ({ ...snapshot.agent }),
    createGroup: async ({ accountStoreId, teamAlias, name, kind }) => {
      const account = stores.find(
        (store) => store.id === accountStoreId && store.kind === 'account',
      );
      if (!account)
        throw failure('store-not-found', 'The account was not found.');
      const id = `team:${teamAlias}`;
      if (stores.some((store) => store.id === id))
        throw failure(
          'conflict',
          'A group with that identifier already exists.',
        );
      stores.push({
        id,
        kind: 'team',
        name: name || teamAlias,
        alias: teamAlias,
        server: account.server,
        account: account.account,
        active: true,
        team_kind: kind,
        team_id_hex: `${kind === 'named' ? '03' : '14'}${'1'.repeat(64)}`,
      });
      parties.push({
        store: id,
        username:
          snapshot.accounts.find((candidate) => candidate.store === account.id)
            ?.username ?? account.account,
        label: 'you',
        party_kind: 'user',
        generation: 1,
        locally_manageable: true,
        party_id_hex: `01${'2'.repeat(64)}`,
        source_role: { role: 'Owner' },
        destination_role: { role: 'Owner' },
      });
      return { applied: true };
    },
    addGroupMember: async ({ storeId, username, destination }) => {
      assertNamedGroup(storeId);
      if (
        parties.some(
          (party) => party.store === storeId && party.username === username,
        )
      )
        throw failure('conflict', 'That user is already in the group.');
      parties.push({
        store: storeId,
        username,
        party_kind: 'user',
        generation: 1,
        locally_manageable: true,
        party_id_hex: `01${String(parties.length + 1).padStart(64, '3')}`,
        source_role: destination,
        destination_role: destination,
      });
      return { applied: true };
    },
    resumeGroupMemberAddition: async ({ storeId }) => {
      assertNamedGroup(storeId);
      return { applied: true };
    },
    demoteGroupMember: async ({ storeId, username, destination }) => {
      assertNamedGroup(storeId);
      const party = parties.find(
        (candidate) =>
          candidate.store === storeId && candidate.username === username,
      );
      if (!party || !party.locally_manageable)
        throw failure(
          'member-not-actionable',
          'That roster party cannot be changed here.',
        );
      const current = parseRole(party.destination_role);
      const next = parseRole(destination);
      const lower =
        roleRank(next) < roleRank(current) ||
        (current?.kind === 'member' &&
          next?.kind === 'member' &&
          visibilityOf(next) < visibilityOf(current));
      if (!lower) throw failure('not-a-demotion', 'Choose a lower role.');
      party.destination_role = destination;
      party.generation += 1;
      return { applied: true };
    },
    removeGroupMember: async ({ storeId, username }) => {
      assertNamedGroup(storeId);
      const party = parties.find(
        (candidate) =>
          candidate.store === storeId && candidate.username === username,
      );
      if (!party || !party.locally_manageable)
        throw failure(
          'member-not-actionable',
          'That roster party cannot be removed here.',
        );
      parties = parties.filter((candidate) => candidate !== party);
      return { applied: true };
    },
    resumeGroupMemberEdit: async (storeId) => {
      assertNamedGroup(storeId);
      return { applied: true };
    },
    admitGroup: async ({ storeId, remoteStoreId, visibility }) => {
      assertNamedGroup(storeId);
      const remote = stores.find(
        (store) => store.id === remoteStoreId && store.kind === 'team',
      );
      const local = stores.find(
        (store) => store.id === storeId && store.kind === 'team',
      );
      if (
        !remote ||
        remote.kind !== 'team' ||
        !remote.active ||
        remote.team_kind !== 'named' ||
        remote.server === local?.server
      )
        throw failure(
          'invalid-request',
          'Choose an active named group on a different server.',
        );
      const operation = `admission-${federation.length + 1}`;
      federation.push({
        store: storeId,
        // Use the profile name rather than the server address.
        remote_profile: remote.server,
        remote_team_alias: remote.alias,
        remote_host_id_hex:
          snapshot.servers.find((server) => server.id === remote.server)
            ?.host_id ?? '',
        remote_team_id_hex: remote.team_id_hex,
        destination: { role: 'Member', visibility },
        operation_id_hex: operation,
        active: true,
      });
      parties.push({
        store: storeId,
        username: null,
        party_kind: 'named-team',
        generation: 1,
        locally_manageable: false,
        party_id_hex: remote.team_id_hex,
        scoped_host_id_hex:
          snapshot.servers.find((server) => server.id === remote.server)
            ?.host_id ?? undefined,
        source_role: { role: 'Owner' },
        destination_role: { role: 'Member', visibility },
      });
      return { applied: true };
    },
    removeFederatedGroup: async ({
      storeId,
      remoteHostIdHex,
      remoteTeamIdHex,
    }) => {
      assertNamedGroup(storeId);
      const matches = federation.filter(
        (candidate) =>
          candidate.store === storeId &&
          candidate.active &&
          candidate.remote_host_id_hex === remoteHostIdHex &&
          candidate.remote_team_id_hex === remoteTeamIdHex,
      );
      if (matches.length !== 1)
        throw failure(
          'invalid-request',
          'Select a single active federated group.',
        );
      federation = federation.filter((candidate) => candidate !== matches[0]);
      parties = parties.filter(
        (candidate) =>
          !(
            candidate.store === storeId &&
            candidate.party_id_hex === remoteTeamIdHex &&
            candidate.scoped_host_id_hex === remoteHostIdHex
          ),
      );
      return { applied: true };
    },
    rerunGroupAdmission: async (storeId, operationId) => {
      assertNamedGroup(storeId);
      const entry = federation.find(
        (candidate) =>
          candidate.store === storeId &&
          candidate.operation_id_hex === operationId,
      );
      if (!entry || entry.active)
        throw failure(
          'admission-not-resumable',
          'This group invitation cannot be resumed.',
        );
      entry.active = true;
      return { applied: true };
    },
    chatLocal: async (action) => {
      if (action.action === 'configure') {
        if (action.enabled !== undefined)
          notificationSettings.enabled = action.enabled;
        if (action.previews !== undefined)
          notificationSettings.previews = action.previews;
        if (action.scope && action.channel) {
          const key = await notificationKey(action.scope, action.channel);
          if (action.mode === null) delete notificationSettings.overrides[key];
          else if (action.mode !== undefined)
            notificationSettings.overrides[key] = action.mode;
        }
      }
      return {
        epoch: '0'.repeat(32),
        available: true,
        settings: {
          ...notificationSettings,
          overrides: { ...notificationSettings.overrides },
        },
      };
    },
    configureWebAdmin: async () => ({ ok: true }),
    openWebAdmin: async () => {
      throw new Error('Host administration requires a connected host.');
    },
    invitation: mockInvitations(stores),
    botAccount: async () => ({
      rows: [],
      message: 'Bot credentials require a connected agent.',
    }),
    setLocalAccountAlias: async (store, label) => {
      if (
        !label ||
        label !== label.trim() ||
        new TextEncoder().encode(label).length > 64 ||
        /[\p{Cc}]/u.test(label)
      )
        throw new Error('Invalid local alias.');
      const account = accounts.find((entry) => entry.store === store);
      if (!account) throw new Error('Account not found.');
      if (
        accounts.some(
          (other) =>
            other.store !== store &&
            other.server === account.server &&
            (other.localAlias ?? other.alias) === label,
        )
      )
        throw new Error('Another account already uses this local alias.');
      account.localAlias = label === account.alias ? undefined : label;
      return { store, alias: label };
    },
    renameAccount: async (_profile, accountAlias, action) =>
      action
        ? [
            {
              operation_id: '2'.repeat(32),
              account_alias: accountAlias,
              state:
                action.action === 'prepare'
                  ? 'prepared'
                  : action.action === 'cancel'
                    ? 'rejected'
                    : 'complete',
              target: action.action === 'prepare' ? action.username : null,
              current_username: null,
              hardware_required: false,
            },
          ]
        : [],
    sso: async (profile, accountAlias, action) => {
      const key = `${profile}/${accountAlias}`;
      const begins =
        action.action === 'begin' || action.action === 'begin-yubi-signup';
      if (begins)
        ssoModes.set(
          key,
          action.action === 'begin' ? action.purpose : 'signup',
        );
      return {
        operationId: action.action === 'account-status' ? null : '1'.repeat(32),
        accountAlias,
        purpose: ssoModes.get(key) ?? 'reauthenticate',
        accountStatus:
          action.action === 'account-status'
            ? {
                state: 'linked',
                rolloutMode: 1,
                providerFence: 0,
                issuer: 'https://identity.example',
                authorizationEpoch: 1,
                authorizationGeneration: 1,
              }
            : null,
        state:
          action.action === 'account-status'
            ? 'linked'
            : begins
              ? 'waiting'
              : action.action === 'poll'
                ? 'ready'
                : action.action === 'cancel'
                  ? 'cancelled'
                  : action.action.startsWith('finish-')
                    ? 'complete'
                    : 'waiting',
        browserAvailable: begins || action.action === 'status',
        expiresAtMs: Date.now() + 600000,
        serviceAccess: action.action.startsWith('finish-'),
      };
    },
    openSsoBrowser: async () => ({ ok: true }),
    openChatLink: async () => ({ ok: true }),
    copyText: async () => ({ ok: true }),
    initializeClientState: async () => ({ state: 'ready' }),
    discoverGoProfiles: async () => ({ installed: false, candidates: [] }),
    checkAndAddProfile: async (profileName, probe) => {
      const path =
        probe === firstRunFixture.invited.server
          ? firstRunFixture.invited
          : probe === firstRunFixture.own.server
            ? firstRunFixture.own
            : undefined;
      if (!path) throw failure('io', `No response from ${probe}`);
      return { ...path.report, profile: profileName };
    },
    checkAndAddGoProfile: async (_candidateId, hostId, profileName, probe) => {
      const path =
        probe === firstRunFixture.invited.server
          ? firstRunFixture.invited
          : probe === firstRunFixture.own.server
            ? firstRunFixture.own
            : undefined;
      if (!path || path.report.hostId !== hostId)
        throw failure('io', `${probe} did not match, so nothing was saved.`);
      return { ...path.report, profile: profileName };
    },
    listPendingOperations: async (profile) =>
      (pending.get(profile) ?? []).map((operation) => ({ ...operation })),
    createFirstRunAccount: async ({ profile, alias, username }) => {
      pending.set(profile, [{ kind: 'account-signup', alias }]);
      ensureFirstRunAccount(profile, alias, username);
      pending.delete(profile);
      return { applied: true };
    },
    resumeFirstRunAccount: async (profile, alias) => {
      const operation = (pending.get(profile) ?? []).find(
        (entry) => entry.kind === 'account-signup' && entry.alias === alias,
      );
      if (!operation)
        throw failure(
          'pending-operation-not-found',
          'That account setup is no longer pending.',
        );
      pending.delete(profile);
      return { applied: true };
    },
    setFirstRunPassphrase: async () => ({ applied: true }),
    prepareOwnerBackup: async (_profile, _accountAlias, backupAlias) => ({
      backupAlias,
      phrase: firstRunFixture.backupPhrase,
    }),
    commitOwnerBackup: async (_profile, _accountAlias, backupAlias, phrase) => {
      if (backupAlias !== 'paper' || phrase !== firstRunFixture.backupPhrase) {
        throw failure('invalid-request', 'The backup phrase does not match.');
      }
      return { applied: true };
    },
    recoverOwnerAccount: async (profile, targetAlias) => {
      pending.set(profile, [{ kind: 'account-recovery', alias: targetAlias }]);
      const path =
        profile === firstRunFixture.invited.profile
          ? firstRunFixture.invited
          : firstRunFixture.own;
      ensureFirstRunAccount(profile, targetAlias, path.username);
      pending.delete(profile);
      return { applied: true };
    },
    resumeOwnerRecovery: async (profile, targetAlias) => {
      const operation = (pending.get(profile) ?? []).find(
        (entry) =>
          entry.kind === 'account-recovery' && entry.alias === targetAlias,
      );
      if (!operation)
        throw failure(
          'pending-operation-not-found',
          'That recovery is no longer pending.',
        );
      const path =
        profile === firstRunFixture.invited.profile
          ? firstRunFixture.invited
          : firstRunFixture.own;
      ensureFirstRunAccount(profile, targetAlias, path.username);
      pending.delete(profile);
      return { applied: true };
    },
    discoverGroups: async (profile, accountAlias) => {
      if (
        profile !== firstRunFixture.invited.profile ||
        accountAlias !== 'sol'
      ) {
        return { accountAlias, groups: [] };
      }
      const engineering = stores.find(
        (store) => store.id === fixtureEngineering?.id,
      );
      if (engineering?.kind === 'team') engineering.account = accountAlias;
      if (
        engineering?.kind === 'team' &&
        !parties.some(
          (party) =>
            party.store === engineering.id && party.username === accountAlias,
        )
      ) {
        parties = [
          ...parties,
          {
            store: engineering.id,
            username: accountAlias,
            label: 'you',
            party_kind: 'user',
            generation: 1,
            locally_manageable: true,
            party_id_hex: `01${'6'.repeat(64)}`,
            source_role: { role: 'Member', visibility: 0 },
            destination_role: { role: 'Member', visibility: 0 },
          },
        ];
      }
      return {
        accountAlias,
        groups: [
          {
            alias: 'engineering',
            accountAlias,
            teamIdHex: fixtureEngineering?.team_id_hex ?? `03${'3'.repeat(64)}`,
            kind: 'named',
            name: 'Engineering',
            active: true,
          },
        ],
      };
    },
    describeServerStatus: async (profile) => {
      const server = servers.find((entry) => entry.id === profile);
      if (!server)
        throw failure('store-not-found', 'That server is not configured.');
      return {
        profile: server.id,
        configuredProbe: serverProbes.get(server.id) ?? server.name,
        host: serverHosts.get(server.id) ?? null,
        leaseRequired: true,
        leaseExpiresAt:
          server.id === 'personal'
            ? Math.floor(Date.now() / 1000) + 6 * 24 * 60 * 60
            : server.id === 'acme'
              ? Math.floor(Date.now() / 1000) - 3 * 24 * 60 * 60
              : serverHosts.has(server.id)
                ? Math.floor(Date.now() / 1000) + 6 * 24 * 60 * 60
                : null,
        chatAvailable: server.capabilities.chat,
      };
    },
    checkServer: async (profile) => {
      const server = servers.find((entry) => entry.id === profile);
      if (!server)
        throw failure('store-not-found', 'That server is not configured.');
      const existing = serverHosts.get(server.id);
      const host = existing ?? {
        lookupName: serverProbes.get(server.id) ?? server.name,
        canonicalName: serverProbes.get(server.id) ?? server.name,
        hostId: hostIds[server.id] ?? `02${'7'.repeat(64)}`,
        chain: 4,
        epoch: 118204,
      };
      serverHosts.set(server.id, host);
      return {
        profile: server.id,
        acceptance: existing ? ('unchanged' as const) : ('inserted' as const),
        serverVersion: {
          minimum: null,
          newest: null,
          message: '',
          compatible: true,
        },
        ...host,
      };
    },
    addServer: async (profileName, probe) => {
      if (servers.some((server) => server.id === profileName))
        throw failure(
          'already-exists',
          'A server with that name already exists.',
        );
      servers.push({
        id: profileName,
        name: profileName,
        label: null,
        configuredProbe: probe,
        host_id: null,
        chain: null,
        epoch: null,
        accounts: [],
        trust: { status: 'unprobed' },
        compatibility: { status: 'required-unavailable' },
        passiveStatus: {
          status: 'available',
          source: 'signed-server-status',
        },
        connectivity: { status: 'unknown' },
        capabilities: { chat: false },
        restrictions: [],
      });
      serverProbes.set(profileName, probe);
      return { profile: profileName, configuredProbe: probe };
    },
    setServerLabel: async (profile, label) => {
      const server = servers.find((entry) => entry.id === profile);
      if (!server)
        throw failure('profile-not-found', 'That server is not configured.');
      if (
        label?.includes('\0') ||
        label?.includes('\r') ||
        label?.includes('\n')
      )
        throw failure('invalid-request', 'Enter a valid display name.');
      const trimmed = label?.trim() ?? '';
      if (new TextEncoder().encode(trimmed).length > 64)
        throw failure('invalid-request', 'Enter a valid display name.');
      const normalized =
        trimmed === '' || trimmed === server.name ? null : trimmed;
      const changed = server.label !== normalized;
      server.label = normalized;
      return { profile, label: normalized, changed };
    },
    forgetServer: async (profile, confirmation) => {
      if (profile !== confirmation)
        throw failure(
          'invalid-request',
          'Type the exact server profile to forget it.',
        );
      const index = servers.findIndex((server) => server.id === profile);
      if (index >= 0) servers.splice(index, 1);
      serverHosts.delete(profile);
      serverProbes.delete(profile);
      return { profile, removed: true as const };
    },
    listAccountDevices: async (accountStoreId) =>
      (deviceRows.get(accountStoreId) ?? []).map((entry) => ({ ...entry })),
    removeAccountDevice: async (accountStoreId, deviceId) => {
      const rows = deviceRows.get(accountStoreId) ?? [];
      const index = rows.findIndex((entry) => entry.id === deviceId);
      if (index >= 0) rows.splice(index, 1);
      return { deviceId, userChainSequence: 14, alreadyAbsent: index < 0 };
    },
    listBackupEnrollments: async (accountStoreId) =>
      (backupRows.get(accountStoreId) ?? []).map((entry) => ({ ...entry })),
    revokeOwnerBackup: async (accountStoreId, backup, confirmation) => {
      if (confirmation !== backup.backupAlias)
        throw failure(
          'invalid-request',
          'Type the exact backup alias to revoke it.',
        );
      const store = accountStore(accountStoreId);
      if (!store || store.account !== backup.accountAlias)
        throw failure('store-not-found', 'That account is not available.');
      const rows = backupRows.get(accountStoreId) ?? [];
      const index = rows.findIndex(
        (entry) => entry.backupAlias === backup.backupAlias,
      );
      if (index >= 0 && rows[index]?.backupId !== backup.backupId)
        throw failure(
          'invalid-request',
          'The backup configuration has changed. Refresh and try again.',
        );
      if (index >= 0) rows.splice(index, 1);
      return {
        ...backup,
        userChainSequence: 15,
        alreadyAbsent: index < 0,
        removedLocalEnrollment: true as const,
      };
    },
    startDevicePairing: async (accountStoreId) => {
      const store = accountStore(accountStoreId);
      if (!store)
        throw failure('store-not-found', 'That account is not available.');
      const phrase = 'cobalt window';
      pairingOffers.set(accountStoreId, phrase);
      return { accountAlias: store.account, phrase };
    },
    resumeDevicePairingOffer: async (accountStoreId) => {
      const store = accountStore(accountStoreId);
      const phrase = pairingOffers.get(accountStoreId);
      if (!store || !phrase)
        throw failure(
          'pending-operation-not-found',
          'This pairing request has expired or already finished.',
        );
      return { accountAlias: store.account, phrase };
    },
    finishDevicePairing: async (accountStoreId) => {
      const store = accountStore(accountStoreId);
      if (!store || !pairingOffers.has(accountStoreId))
        throw failure(
          'pending-operation-not-found',
          'Device pairing has not been completed yet.',
        );
      pairingOffers.delete(accountStoreId);
      return {
        alias: store.account,
        deviceId: `04${'5'.repeat(64)}`,
        userChainSequence: 15,
      };
    },
    acceptDevicePairing: async (profile, targetAlias) => {
      pairingAcceptances.add(`${profile}:${targetAlias}`);
      return {
        alias: targetAlias,
        deviceId: `04${'6'.repeat(64)}`,
        userChainSequence: 15,
      };
    },
    acceptGoProfilePairing: async (_candidateId, profile, targetAlias) => {
      pairingAcceptances.add(`${profile}:${targetAlias}`);
      return {
        alias: targetAlias,
        deviceId: `04${'6'.repeat(64)}`,
        userChainSequence: 15,
      };
    },
    resumeDevicePairingAcceptance: async (profile, targetAlias) => {
      if (!pairingAcceptances.has(`${profile}:${targetAlias}`))
        throw failure(
          'pending-operation-not-found',
          'That pairing acceptance is no longer pending.',
        );
      pairingAcceptances.delete(`${profile}:${targetAlias}`);
      return {
        alias: targetAlias,
        deviceId: `04${'6'.repeat(64)}`,
        userChainSequence: 15,
      };
    },
    resumeGoProfilePairing: async (_candidateId, profile, targetAlias) => {
      if (!pairingAcceptances.has(`${profile}:${targetAlias}`))
        throw failure(
          'pending-operation-not-found',
          'That pairing acceptance is no longer pending.',
        );
      pairingAcceptances.delete(`${profile}:${targetAlias}`);
      return {
        alias: targetAlias,
        deviceId: `04${'6'.repeat(64)}`,
        userChainSequence: 15,
      };
    },
    copyGoProfileDevice: async (_candidateId, _profile, targetAlias) => ({
      alias: targetAlias,
      deviceId: `04${'7'.repeat(64)}`,
      userChainSequence: 15,
    }),
    setAccountPassphrase: async () => ({
      generation: 1,
      stretchVersion: 'v1' as const,
      verified: true as const,
    }),
    changeAccountPassphrase: async () => ({
      generation: 2,
      stretchVersion: 'v1' as const,
      verified: true as const,
    }),
    verifyAccountPassphrase: async () => ({
      generation: 2,
      stretchVersion: 'v1' as const,
      verified: true as const,
    }),
    describeReset: async (profile) => {
      const token = `reset-${profile}-${Date.now()}`;
      resetTokens.add(token);
      return {
        profile,
        resumables:
          profile === 'personal'
            ? [{ kind: 'team-creation' as const, alias: 'homelab' }]
            : [],
        artifacts: [{ kind: 'catalog-cache', entries: 5, bytes: 8192 }],
        token,
        expiresInSeconds: 60,
      };
    },
    resetServer: async (profile, confirmation, token) => {
      if (profile !== confirmation || !resetTokens.delete(token))
        throw failure(
          'invalid-request',
          'Review the reset details and enter the exact profile name to confirm.',
        );
      return { applied: true };
    },
    listYubiCards: async () =>
      snapshot.cardsConnected.map((card) => ({ ...card })),
    listYubiAccounts: async (profile) =>
      yubi
        .filter((entry) => entry.server === profile)
        .map((entry) => ({
          alias: entry.alias,
          state: entry.state,
          cardSerial: entry.serial,
        })),
    runYubi: async ({ command, args }) => {
      if (
        command === 'yubi_pin_status' ||
        command === 'change_yubi_pin' ||
        command === 'unblock_yubi_pin'
      )
        return { remaining: 3, blocked: false };
      if (command.includes('passphrase'))
        return { generation: 2, stretchVersion: 'v1', verified: true };
      const alias =
        'alias' in args && typeof args.alias === 'string'
          ? args.alias
          : 'yubiAlias' in args && typeof args.yubiAlias === 'string'
            ? args.yubiAlias
            : 'primary key';
      if (command === 'create_yubi_account')
        yubi.push({
          alias,
          server:
            'profile' in args && typeof args.profile === 'string'
              ? args.profile
              : '',
          state: 'complete',
        });
      if (
        command === 'create_yubi_account' ||
        command === 'resume_yubi_account' ||
        command === 'provision_yubi_device'
      ) {
        return {
          alias,
          username: 'satoshi',
          yubiId: `08${'8'.repeat(66)}`,
          subkeyId: `0d${'d'.repeat(64)}`,
          userChainSequence: 22,
          managementEnrolled: true,
        };
      }
      if (command === 'sync_yubi_account')
        return {
          username: 'satoshi',
          userChainSequence: 22,
          directories: 2,
          entries: 7,
          federation: [],
        };
      if (command === 'change_yubi_puk') return { alias, changed: true };
      if (command === 'recover_yubi_subkey')
        return { alias, subkeyId: `0d${'d'.repeat(64)}`, certificateCount: 2 };
      if (command === 'revoke_yubi_device')
        return { alias, userChainSequence: 23, removedLocalCredential: true };
      return { alias, managementEnrolled: true, managementGeneration: 2 };
    },
    onDropHover: async (listener) => {
      hoverListeners.add(listener);
      return () => hoverListeners.delete(listener);
    },
    onDropPaths: async (listener) => {
      pathListeners.add(listener);
      return () => pathListeners.delete(listener);
    },
    onWindowState: async () => () => {},
    onChatNotification: async () => () => {},
    onOpenSettings: async () => () => {},
    onMaintenanceStatus: async () => () => {},
  };
}
