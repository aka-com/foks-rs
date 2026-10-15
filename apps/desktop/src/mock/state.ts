/**
 * The records the mock bridge's domains share: one copy of the fixture
 * snapshot's stores, servers, accounts, items and team rosters, which each
 * domain in src/mock/ reads and changes.
 */

import type { Bridge, FirstRunFixture, StoredHost } from '../bridge';
import { decodeFirstRunCheckpoint } from '../first-run-state';
import { itemKey } from '../model/types';
import type {
  Account,
  AgentSnapshot,
  FederationEntry,
  Item,
  Party,
  Server,
  Store,
} from '../model/types';
import { readStorage, STORAGE_KEYS } from '../platform/device-storage';

/**
 * A CommandError as the host reports one: the code is also its kind, and
 * only `agent-lost` is retryable and fatal.
 */
export const failure = (code: string, message: string) => ({
  code,
  message,
  retryable: code === 'agent-lost',
  ambiguous: false,
  fatal: code === 'agent-lost',
  details: { kind: code },
});

/**
 * The members a `bridge/commands-*.ts` factory supplies, each one required, as
 * a mock domain module supplies them. A native-only optional method is set to
 * `undefined` rather than left out, so the mock bridge's override check sees
 * every member.
 */
export type MockDomain<Factory extends (...args: never[]) => object> = {
  [K in Extract<keyof ReturnType<Factory>, keyof Bridge>]: Bridge[K];
};

/** The host ID the mock pins for each fixture profile it checks. */
export const HOST_IDS: Readonly<Record<string, string>> = {
  personal: `02${'9f31c2aa'.repeat(8)}`,
  acme: `02${'b04d17e3'.repeat(8)}`,
  partner: `02${'04c8b19e'.repeat(8)}`,
};

export interface MockState {
  readonly snapshot: AgentSnapshot;
  readonly stores: Store[];
  readonly servers: Server[];
  /** The address each profile was added with, by profile name. */
  readonly serverProbes: Map<string, string>;
  /** The host each checked profile is bound to, by profile name. */
  readonly serverHosts: Map<string, StoredHost>;
  readonly accounts: Account[];
  items: Item[];
  parties: Party[];
  federation: FederationEntry[];
  /** Each item's plaintext, by `itemKey`. */
  readonly contents: Map<string, string>;
  /**
   * The host discards a store's catalog items when it writes to the store,
   * so a selection in that store fails until the catalog is read again.
   */
  readonly unread: Set<string>;
  readonly firstRunFixture: FirstRunFixture;
  /**
   * Adds the account a saved first-run checkpoint names, so a mocked
   * quit and resume finds the account setup created.
   */
  readonly restoreFirstRunAccount: () => void;
  /** Adds or updates the account store a first-run step created. */
  readonly ensureFirstRunAccount: (
    profile: string,
    alias: string,
    username: string,
  ) => void;
}

export function mockState(snapshot: AgentSnapshot): MockState {
  const stores: Store[] = snapshot.stores.map((store) => ({ ...store }));
  const servers = snapshot.servers.map((server) => ({ ...server }));
  const serverProbes = new Map(
    snapshot.servers.map((server) => [
      server.profileName,
      server.configuredEndpoint,
    ]),
  );
  const accounts = snapshot.accounts.map((account) => ({ ...account }));
  const items = snapshot.items.map((item) => ({ ...item }));
  const contents = new Map(
    items.map((item) => [
      itemKey(item),
      snapshot.plaintext[itemKey(item)] ?? '',
    ]),
  );
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
    // HESP shape: nine BIP-39 words alternating with eight numbers.
    recoveryPhrase:
      'orbit 4071 velvet 128 cactus 16 mirror 2077 harbor 64 pistol 7310 thumb 512 copper 3306 ivory',
  };
  const serverHosts = new Map<string, StoredHost>();
  for (const server of servers) {
    if (server.host_id && server.chain !== null && server.epoch !== null) {
      serverHosts.set(server.profileName, {
        lookupName: server.configuredEndpoint,
        canonicalName: server.configuredEndpoint,
        hostId: HOST_IDS[server.profileName] ?? `02${'1'.repeat(64)}`,
        chain: server.chain,
        epoch: server.epoch,
      });
    }
  }
  const restoreFirstRunAccount = (): void => {
    if (typeof window === 'undefined') return;
    const saved = decodeFirstRunCheckpoint(readStorage(STORAGE_KEYS.firstRun));
    if (!saved?.profile) return;
    // The preview simulates persisted native state after a successful mutation.
    // Keep its host binding and profile IDs consistent with the real DTOs.
    const server = servers.find(
      (entry) => entry.profileName === saved.profile?.profile,
    );
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
  return {
    snapshot,
    stores,
    servers,
    serverProbes,
    serverHosts,
    accounts,
    items,
    parties: snapshot.parties.map((party) => ({ ...party })),
    federation: snapshot.federation.map((entry) => ({ ...entry })),
    contents,
    unread: new Set<string>(),
    firstRunFixture,
    restoreFirstRunAccount,
    ensureFirstRunAccount,
  };
}
