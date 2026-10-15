/**
 * The mock's account and team commands, the members of
 * `bridge/commands-accounts.ts`, over the shared store and roster records.
 * The account listing and the team changes answer at the wire level through
 * the mock host; the other members answer with decoded values.
 */

import type {
  AddFederatedTeamMemberRequest,
  CreateGroupRequest,
  GroupMemberRequest,
  GroupRoleRequest,
  RemoveFederatedGroupRequest,
} from '../bridge';
import type { accountCommands } from '../bridge/commands-accounts';
import { parseRole, roleRank, visibilityOf } from '../model/roles';
import type { Store, StoreRef } from '../model/types';
import type { RenameProgress } from '../rename-contract';
import type { SsoPurpose } from '../sso-contract';
import { mockCommand, type MockCommand } from './host';
import { failure, type MockDomain, type MockState } from './state';
import { accountWire } from './wire';

/**
 * The account and team members whose commands the mock host answers at the
 * wire level, through `bridge/commands-accounts.ts`.
 */
export const ACCOUNT_HOST_MEMBERS = [
  'listAccounts',
  'resumeGroupCreation',
  'abandonGroupCreation',
  'createGroup',
  'addGroupMember',
  'resumeGroupMemberAddition',
  'promoteGroupMember',
  'demoteGroupMember',
  'removeGroupMember',
  'resumeGroupMemberEdit',
  'addFederatedTeamMember',
  'removeFederatedGroup',
  'rerunFederatedTeamMemberAdd',
] as const satisfies readonly (keyof ReturnType<typeof accountCommands>)[];

/**
 * Throws the failure the host reports when `storeId` is not an active named
 * team.
 */
function assertNamedGroup(stores: readonly Store[], storeId: string): void {
  const store = stores.find((candidate) => candidate.id === storeId);
  if (!store || store.kind !== 'team')
    throw failure('store-not-found', 'The team was not found.');
  if (!store.active)
    throw failure('inactive-group', 'The team setup is incomplete.');
  if (store.team_kind !== 'named')
    throw failure(
      'group-management-unavailable',
      'Managing members and shared access requires a named team.',
    );
}

/**
 * The commands of `ACCOUNT_HOST_MEMBERS`: the account listing and the team
 * roster and federation changes, answered with wire JSON.
 */
export function mockAccountHost(state: MockState): Record<string, MockCommand> {
  const { snapshot, stores, accounts, restoreFirstRunAccount } = state;
  return {
    list_accounts: () => {
      restoreFirstRunAccount();
      // Fixture and restored accounts already map directly to their store IDs.
      return accounts.map(accountWire);
    },
    resume_group_creation: mockCommand(
      async ({ storeId }: { storeId: StoreRef }) => {
        const store = stores.find((candidate) => candidate.id === storeId);
        if (!store || store.kind !== 'team' || store.active) {
          throw failure(
            'invalid-request',
            'This team has already completed setup.',
          );
        }
        store.active = true;
        return { applied: true };
      },
    ),
    abandon_group_creation: mockCommand(
      async ({ storeId }: { storeId: StoreRef }) => {
        const index = stores.findIndex((candidate) => candidate.id === storeId);
        const store = index < 0 ? undefined : stores[index];
        if (!store || store.kind !== 'team' || store.active !== false) {
          throw failure(
            'invalid-request',
            'This team has no incomplete creation to remove.',
          );
        }
        stores.splice(index, 1);
        return { applied: true };
      },
    ),
    create_group: mockCommand(
      async ({ accountStoreId, teamAlias, name, kind }: CreateGroupRequest) => {
        const account = stores.find(
          (store) => store.id === accountStoreId && store.kind === 'account',
        );
        if (!account)
          throw failure('store-not-found', 'The account was not found.');
        const id = `team:${teamAlias}`;
        if (stores.some((store) => store.id === id))
          throw failure(
            'conflict',
            'A team with that identifier already exists.',
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
        state.parties.push({
          store: id,
          username:
            snapshot.accounts.find(
              (candidate) => candidate.store === account.id,
            )?.username ?? account.account,
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
    ),
    add_group_member: mockCommand(
      async ({ storeId, username, destination }: GroupRoleRequest) => {
        assertNamedGroup(stores, storeId);
        if (
          state.parties.some(
            (party) => party.store === storeId && party.username === username,
          )
        )
          throw failure('conflict', 'That user is already in the team.');
        state.parties.push({
          store: storeId,
          username,
          party_kind: 'user',
          generation: 1,
          locally_manageable: true,
          party_id_hex: `01${String(state.parties.length + 1).padStart(64, '3')}`,
          source_role: destination,
          destination_role: destination,
        });
        return { applied: true };
      },
    ),
    resume_group_member_addition: mockCommand(
      async ({ storeId }: GroupMemberRequest) => {
        assertNamedGroup(stores, storeId);
        return { applied: true };
      },
    ),
    promote_group_member: mockCommand(
      async ({ storeId, username, destination }: GroupRoleRequest) => {
        assertNamedGroup(stores, storeId);
        const party = state.parties.find(
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
        const higher =
          roleRank(next) > roleRank(current) ||
          (current?.kind === 'member' &&
            next?.kind === 'member' &&
            visibilityOf(next) > visibilityOf(current));
        if (!higher) throw failure('not-a-promotion', 'Choose a higher role.');
        party.destination_role = destination;
        return { applied: true };
      },
    ),
    demote_group_member: mockCommand(
      async ({ storeId, username, destination }: GroupRoleRequest) => {
        assertNamedGroup(stores, storeId);
        const party = state.parties.find(
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
    ),
    remove_group_member: mockCommand(
      async ({ storeId, username }: GroupMemberRequest) => {
        assertNamedGroup(stores, storeId);
        const party = state.parties.find(
          (candidate) =>
            candidate.store === storeId && candidate.username === username,
        );
        if (!party || !party.locally_manageable)
          throw failure(
            'member-not-actionable',
            'That roster party cannot be removed here.',
          );
        state.parties = state.parties.filter(
          (candidate) => candidate !== party,
        );
        return { applied: true };
      },
    ),
    resume_group_member_edit: mockCommand(
      async ({ storeId }: { storeId: StoreRef }) => {
        assertNamedGroup(stores, storeId);
        return { applied: true };
      },
    ),
    add_federated_team_member: mockCommand(
      async ({
        storeId,
        remoteStoreId,
        visibility,
      }: AddFederatedTeamMemberRequest) => {
        assertNamedGroup(stores, storeId);
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
            'Choose an active named team on a different server.',
          );
        const operation = `federated-member-${state.federation.length + 1}`;
        state.federation.push({
          store: storeId,
          // Use the profile name rather than the server address.
          remote_profile: remote.server,
          remote_team_alias: remote.alias,
          remote_host_id_hex:
            snapshot.servers.find(
              (server) => server.profileName === remote.server,
            )?.host_id ?? '',
          remote_team_id_hex: remote.team_id_hex,
          destination: { role: 'Member', visibility },
          operation_id_hex: operation,
          active: true,
        });
        state.parties.push({
          store: storeId,
          username: null,
          party_kind: 'named-team',
          generation: 1,
          locally_manageable: false,
          party_id_hex: remote.team_id_hex,
          scoped_host_id_hex:
            snapshot.servers.find(
              (server) => server.profileName === remote.server,
            )?.host_id ?? undefined,
          source_role: { role: 'Owner' },
          destination_role: { role: 'Member', visibility },
        });
        return { applied: true };
      },
    ),
    expel_federated_group: mockCommand(
      async ({
        storeId,
        remoteHostIdHex,
        remoteTeamIdHex,
      }: RemoveFederatedGroupRequest) => {
        assertNamedGroup(stores, storeId);
        const matches = state.federation.filter(
          (candidate) =>
            candidate.store === storeId &&
            candidate.active &&
            candidate.remote_host_id_hex === remoteHostIdHex &&
            candidate.remote_team_id_hex === remoteTeamIdHex,
        );
        if (matches.length !== 1)
          throw failure(
            'invalid-request',
            'Select a single active federated team.',
          );
        state.federation = state.federation.filter(
          (candidate) => candidate !== matches[0],
        );
        state.parties = state.parties.filter(
          (candidate) =>
            !(
              candidate.store === storeId &&
              candidate.party_id_hex === remoteTeamIdHex &&
              candidate.scoped_host_id_hex === remoteHostIdHex
            ),
        );
        return { applied: true };
      },
    ),
    rerun_federated_team_member_add: mockCommand(
      async ({
        storeId,
        operationId,
      }: {
        storeId: StoreRef;
        operationId: string;
      }) => {
        assertNamedGroup(stores, storeId);
        const entry = state.federation.find(
          (candidate) =>
            candidate.store === storeId &&
            candidate.operation_id_hex === operationId,
        );
        if (!entry || entry.active)
          throw failure(
            'federated-membership-not-resumable',
            'This team invitation cannot be resumed.',
          );
        entry.active = true;
        return { applied: true };
      },
    ),
  };
}

export function mockAccountCommands(
  state: MockState,
): MockDomain<typeof accountCommands, (typeof ACCOUNT_HOST_MEMBERS)[number]> {
  const { snapshot, stores, accounts, firstRunFixture } = state;
  const ssoModes = new Map<string, SsoPurpose>();
  const renames = new Map<string, RenameProgress[]>();
  const fixtureEngineeringStore = snapshot.stores.find(
    (store) => store.id === 'team:eng',
  );
  const fixtureEngineering =
    fixtureEngineeringStore?.kind === 'team'
      ? fixtureEngineeringStore
      : undefined;
  return {
    listGroupDetails: (storeId) =>
      Promise.resolve({
        parties: {
          status: 'success' as const,
          value: state.parties
            .filter((party) => party.store === storeId)
            .map((party) => ({ ...party })),
        },
        federation: {
          status: 'success' as const,
          value: state.federation
            .filter((entry) => entry.store === storeId)
            .map((entry) => ({ ...entry })),
        },
      }),
    listParties: (storeId) =>
      Promise.resolve(
        state.parties
          .filter((party) => party.store === storeId)
          .map((party) => ({ ...party })),
      ),
    listFederation: (storeId) =>
      Promise.resolve(
        state.federation
          .filter((entry) => entry.store === storeId)
          .map((entry) => ({ ...entry })),
      ),
    configureWebAdmin: async () => ({ ok: true }),
    openWebAdmin: async () => {
      throw new Error('Host administration requires a connected host.');
    },
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
    // Persist mock operations across calls. Listings omit target usernames.
    renameAccount: async (profile, accountAlias, action) => {
      const key = `${profile}/${accountAlias}`;
      const rows = renames.get(key) ?? [];
      if (!action)
        return rows.map((row) => ({
          ...row,
          target: null,
          current_username: null,
        }));
      if (action.action === 'prepare') {
        if (rows.some((row) => !['complete', 'rejected'].includes(row.state)))
          throw new Error(
            'A username change is already pending. Confirm or discard it first.',
          );
        const row: RenameProgress = {
          operation_id: (renames.size + rows.length + 2)
            .toString(16)
            .padStart(32, '0'),
          account_alias: accountAlias,
          state: 'prepared',
          target: action.username,
          current_username:
            accounts.find(
              (entry) =>
                entry.server === profile && entry.alias === accountAlias,
            )?.username ?? null,
          hardware_required: false,
        };
        renames.set(key, [row, ...rows]);
        return [{ ...row }];
      }
      const row = rows.find(
        (entry) => entry.operation_id === action.operation_id,
      );
      if (!row) throw new Error('Rename operation not found.');
      if (action.action === 'cancel') row.state = 'rejected';
      else if (action.action === 'attempt' && row.state === 'prepared')
        row.state = 'complete';
      return [{ ...row }];
    },
    // Development data has no agent clock to disagree with.
    adapterClock: async (profile, accountAlias) => {
      if (
        !accounts.some(
          (entry) => entry.server === profile && entry.alias === accountAlias,
        )
      )
        throw new Error('Account not found.');
      return {
        admission: 'trusted',
        localTime: Math.floor(Date.now() / 1_000),
        resumesAt: null,
      };
    },
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
                providerBlockedReason: 0,
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
        !state.parties.some(
          (party) =>
            party.store === engineering.id && party.username === accountAlias,
        )
      ) {
        state.parties = [
          ...state.parties,
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
  };
}
