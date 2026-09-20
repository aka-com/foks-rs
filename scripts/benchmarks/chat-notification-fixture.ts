import type { ChatScope } from '../../apps/desktop/src/chat-contract';
import type { AgentSnapshot } from '../../apps/desktop/src/model';

/** Presentation metadata for the authenticated, isolated V019 worker fixture. */
export function notificationBenchmarkSnapshot(
  storeId: string,
  scope: ChatScope,
): AgentSnapshot {
  const profile = scope.store.profile;
  return {
    agent: { state: 'ready' },
    stores: [
      {
        id: storeId,
        kind: 'team',
        name: 'Benchmark',
        alias: scope.store.team_alias,
        server: profile,
        account: scope.store.account_alias,
        active: true,
        team_kind: 'named',
        team_id_hex: scope.store.team_id,
      },
    ],
    servers: [
      {
        id: profile,
        name: 'Isolated benchmark',
        label: null,
        configuredProbe: 'isolated-benchmark',
        host_id: scope.host,
        chain: null,
        epoch: null,
        accounts: [scope.store.account_alias],
        trust: { status: 'verified' },
        compatibility: { status: 'not-required' },
        passiveStatus: { status: 'available', source: 'signed-server-status' },
        connectivity: { status: 'unknown' },
        services: { chat: true },
        restrictions: [],
      },
    ],
    storeInventory: [{ store: storeId, status: 'available', restrictions: [] }],
    profileInventory: [{ profile, accounts: 'complete', teams: 'complete' }],
    catalogProfiles: [profile],
    profileInventoryStatus: 'complete',
    accounts: [],
    items: [],
    parties: [],
    federation: [],
    groupDetailFailures: [],
    devices: [],
    yubiAccounts: [],
    cardsConnected: [],
    notifications: [],
    observedExpiredLeases: [],
    plaintext: {},
  };
}
