/**
 * Mock data fixture representing servers, stores, parties, and items for
 * local development and testing.
 */

import { applyLease } from './model/lease';
import type { AgentSnapshot } from './model/types';

/** Base fixture data prior to applying lease configuration. */
const RAW: AgentSnapshot = {
  agent: { state: 'ready' },
  servers: [
    {
      id: 'personal',
      name: 'foks.example.net',
      label: 'Personal server',
      configuredProbe: 'foks.example.net',
      host_id: '9f31c2aa07',
      chain: 12,
      epoch: 4821,
      accounts: ['personal'],
      trust: { status: 'verified' },
      compatibility: { status: 'not-required' },
      passiveStatus: { status: 'available', source: 'signed-server-status' },
      connectivity: { status: 'unknown' },
      capabilities: { chat: true },
      restrictions: [],
    },
    {
      id: 'acme',
      name: 'foks.acme-corp.com',
      label: 'Acme',
      configuredProbe: 'foks.acme-corp.com',
      host_id: 'b04d17e390',
      chain: 33,
      epoch: 90417,
      accounts: ['work'],
      trust: { status: 'verified' },
      compatibility: { status: 'required', expiresAt: 0 },
      passiveStatus: { status: 'available', source: 'signed-server-status' },
      connectivity: { status: 'unknown' },
      capabilities: { chat: false },
      restrictions: [],
    },
    {
      id: 'partner',
      name: 'foks.partner.dev',
      label: null,
      configuredProbe: 'foks.partner.dev',
      host_id: null,
      chain: null,
      epoch: null,
      accounts: [],
      trust: { status: 'unprobed' },
      compatibility: { status: 'not-required' },
      passiveStatus: { status: 'available', source: 'signed-server-status' },
      connectivity: { status: 'unknown' },
      capabilities: { chat: false },
      restrictions: [],
    },
  ],
  // `store` is the exact identity and matches the `stores` entry below; the
  // alias is only what the row is labelled with.
  accounts: [
    {
      store: 'acct:personal',
      alias: 'personal',
      username: 'satoshi',
      server: 'personal',
    },
    { store: 'acct:work', alias: 'work', username: 'vitalik', server: 'acme' },
  ],
  storeInventory: [],
  profileInventory: [
    { profile: 'personal', accounts: 'complete', teams: 'complete' },
    { profile: 'acme', accounts: 'complete', teams: 'complete' },
    { profile: 'partner', accounts: 'complete', teams: 'complete' },
  ],
  catalogProfiles: ['personal', 'acme'],
  profileInventoryStatus: 'complete',
  stores: [
    {
      id: 'acct:personal',
      kind: 'account',
      name: 'Personal',
      server: 'personal',
      account: 'personal',
    },
    {
      id: 'acct:work',
      kind: 'account',
      name: 'Work (Acme)',
      server: 'acme',
      account: 'work',
    },
    {
      id: 'team:eng',
      kind: 'team',
      name: 'Engineering',
      alias: 'engineering',
      server: 'acme',
      account: 'work',
      active: true,
      team_kind: 'named',
      team_id_hex:
        '033f91c2aa07b45e18d0c73a9f2e5b6417ac8d0192f3e4b5c6d7089a1b2c3d4e5f',
    },
    {
      id: 'team:household',
      kind: 'team',
      name: 'Household',
      alias: 'household',
      server: 'personal',
      account: 'personal',
      active: true,
      team_kind: 'named',
      team_id_hex:
        '037b83d4f0241a67933c59ed218458ce03a5b98e691e81c544b8cc93c52f4e02ab',
    },
    {
      id: 'team:homelab',
      kind: 'team',
      name: 'Homelab',
      alias: 'homelab',
      server: 'personal',
      account: 'personal',
      active: false,
      creation_phase: 'remote-verified',
      team_kind: 'adhoc',
      team_id_hex:
        '038b17e4a055c93d2fe60148ab7f2c9d3140e5b8a71c6f93d20ba48e5739d1c0f6',
    },
  ],
  items: [
    {
      store: 'acct:personal',
      path: '/logins/github.com',
      kind: 'Secret',
      size: 142,
      version: 9,
      read: 'Owner',
      write: 'Owner',
      value:
        'username: satoshi\npassword: ••••••••••••\nurl: https://github.com/login',
    },
    {
      store: 'acct:personal',
      path: '/logins/fastmail.com',
      kind: 'Secret',
      size: 96,
      version: 2,
      read: 'Owner',
      write: 'Owner',
      value: 'user: satoshi@fastmail.com\npassword: ••••••••••',
    },
    {
      store: 'acct:personal',
      path: '/env/prod/DATABASE_URL',
      kind: 'Secret',
      size: 96,
      version: 3,
      read: 'Owner',
      write: 'Owner',
      value: 'postgres://service:••••••••@db.internal/prod',
    },
    {
      store: 'acct:personal',
      path: '/ssh/id_ed25519',
      kind: 'File',
      size: 419,
      version: 1,
      read: 'Owner',
      write: 'Owner',
    },
    {
      store: 'acct:personal',
      path: '/ssh',
      kind: 'Folder',
      size: 0,
      version: 1,
      read: 'Owner',
      write: 'Owner',
    },
    {
      store: 'acct:personal',
      path: '/documents/passport-scan.pdf',
      kind: 'File',
      size: 2841992,
      version: 1,
      read: 'Owner',
      write: 'Owner',
    },
    {
      store: 'acct:personal',
      path: '/agents/anthropic-api-key',
      kind: 'Secret',
      size: 108,
      version: 4,
      read: 'Owner',
      write: 'Owner',
      value: 'sk-ant-••••••••••••••••',
    },
    {
      store: 'team:eng',
      path: '/deploy/production-token',
      kind: 'Secret',
      size: 88,
      version: 12,
      read: 'Admin',
      write: 'Owner',
      value: 'foks_team_token_••••••••',
    },
    {
      store: 'team:eng',
      path: '/deploy/staging-token',
      kind: 'Secret',
      size: 88,
      version: 3,
      read: 'Member · visibility 0',
      write: 'Admin',
      value: 'foks_team_token_••••••••',
    },
    {
      store: 'team:eng',
      path: '/release/bundle.tar',
      kind: 'File',
      size: 84399718,
      version: 5,
      read: 'Member · visibility 0',
      write: 'Admin',
    },
    {
      store: 'team:eng',
      path: '/onboarding/README.md',
      kind: 'File',
      size: 5120,
      version: 2,
      read: 'Member · visibility 0',
      write: 'Admin',
    },
    {
      store: 'team:household',
      path: '/wifi/guest-password',
      kind: 'Secret',
      size: 64,
      version: 4,
      read: 'Member · visibility 0',
      write: 'Admin',
      value: 'ssid: Satoshi-Guest\npassword: ••••••••',
    },
    {
      store: 'team:household',
      path: '/documents/emergency.pdf',
      kind: 'File',
      size: 2841992,
      version: 7,
      read: 'Member · visibility 0',
      write: 'Owner',
    },
    {
      store: 'team:household',
      path: '/streaming/netflix',
      kind: 'Secret',
      size: 70,
      version: 1,
      read: 'Member · visibility 0',
      write: 'Admin',
      value: 'user: family@example.net\npassword: ••••••••',
    },
  ],
  parties: [
    {
      store: 'team:eng',
      username: 'sam.ortiz',
      party_kind: 'user',
      generation: 4,
      locally_manageable: true,
      party_id_hex:
        '01a4c107f822ee91b05d3c44a7e0186b297f10cd439b62ae0831d5f7c46e29b0a5',
      source_role: { role: 'Owner' },
      destination_role: { role: 'Owner' },
    },
    {
      store: 'team:eng',
      username: 'vitalik',
      label: 'you',
      party_kind: 'user',
      generation: 6,
      locally_manageable: true,
      party_id_hex:
        '016ab09cd4e11f207877c31b900b44aa193d90c1e45f22c8d13e91c7a29c04e211',
      source_role: { role: 'Admin' },
      destination_role: { role: 'Admin' },
    },
    // Engineering's second Admin prevents an accidental one-person count.
    {
      store: 'team:eng',
      username: 'priya.n',
      party_kind: 'user',
      generation: 5,
      locally_manageable: true,
      party_id_hex:
        '0184f0c2a71b3d95e6082c47fa19b6d3e05c84719fb2a6c308d5e71f94c3b0d726',
      source_role: { role: 'Admin' },
      destination_role: { role: 'Admin' },
    },
    {
      store: 'team:eng',
      username: 'dana.okafor',
      party_kind: 'user',
      generation: 5,
      locally_manageable: true,
      party_id_hex:
        '012f8b6d13c07a4e5991b2f8d04a6c3e17bd50927fe83a1c4670d9b2e518f4a06c',
      source_role: { role: 'Member', visibility: 0 },
      destination_role: { role: 'Member', visibility: 0 },
    },
    // A member group is not a user of this server.
    {
      store: 'team:eng',
      username: null,
      party_kind: 'named-team',
      team_name: 'homelab @ foks.example.net',
      generation: 3,
      locally_manageable: false,
      party_id_hex:
        '038b17e4a055c93d2fe60148ab7f2c9d3140e5b8a71c6f93d20ba48e5739d1c0f6',
      scoped_host_id_hex: '9f31c2aa07',
      source_role: { role: 'Owner' },
      destination_role: { role: 'Member', visibility: 0 },
      note: 'Federated team: manage permissions in Engineering under Members › Teams on other servers.',
    },
    // A service account is a user like any other.
    {
      store: 'team:eng',
      username: 'deploy-bot',
      party_kind: 'user',
      generation: 6,
      locally_manageable: true,
      party_id_hex:
        '01c93d2f8b17e4a0148ab7f2e60155c9a71c6f93d23140e5b839d1c0f60ba48e57',
      source_role: { role: 'Member', visibility: 0 },
      destination_role: { role: 'Member', visibility: 0 },
      note: 'Service account managed directly on this server.',
    },
    {
      store: 'team:household',
      username: 'satoshi',
      label: 'you',
      party_kind: 'user',
      generation: 2,
      locally_manageable: true,
      party_id_hex:
        '016ab09cd4e11f207877c31b900b44aa193d90c1e45f22c8d13e91c7a29c04e211',
      source_role: { role: 'Owner' },
      destination_role: { role: 'Owner' },
    },
    {
      store: 'team:household',
      username: 'sam',
      party_kind: 'user',
      generation: 2,
      locally_manageable: true,
      party_id_hex:
        '01a4c107f822ee91b05d3c44a7e0186b297f10cd439b62ae0831d5f7c46e29b0a5',
      source_role: { role: 'Member', visibility: 0 },
      destination_role: { role: 'Member', visibility: 0 },
    },
  ],
  groupDetailFailures: [],
  federation: [
    {
      store: 'team:eng',
      remote_profile: 'personal',
      remote_team_alias: 'homelab',
      remote_host_id_hex: '9f31c2aa07',
      remote_team_id_hex:
        '038b17e4a055c93d2fe60148ab7f2c9d3140e5b8a71c6f93d20ba48e5739d1c0f6',
      destination: { role: 'Member', visibility: 0 },
      operation_id_hex: '7c14a9f0',
      active: false,
    },
  ],
  devices: [
    {
      alias: 'personal',
      name: 'MacBook Pro',
      role: 'owner',
      current: true,
      id_hex:
        '02a779c40674942e2d0fc18aa8d59b2ee4fa95ea7a652310e3f13d2d317f170b22',
    },
    {
      alias: 'laptop',
      name: 'Travel Mac',
      role: 'owner',
      current: false,
      id_hex:
        '02c1e08d5f3a94b7d21e6f0c8a3b5d7e9f1a2b3c4d5e6f708192a3b4c5d6e7f809',
    },
  ],
  yubiAccounts: [
    {
      alias: 'primary key',
      server: 'personal',
      serial: 20993145,
      state: 'complete',
    },
    // An enrollment the agent never finished: the work account's key list is
    // one row with the Incomplete chip and a revoke that cannot run.
    {
      alias: 'work key',
      server: 'acme',
      serial: 20993146,
      state: 'pending',
    },
  ],
  cardsConnected: [{ serial: 20993145 }],
  notifications: [
    {
      id: 'lease-acme',
      severity: 'crit',
      title: 'Acme is locked',
      detail:
        'The server’s check-in expired. Work and Engineering teams are unavailable until the agent renews it.',
      action: 'Check status',
    },
    {
      id: 'team-homelab',
      severity: 'warn',
      title: 'Homelab is inactive',
      detail:
        'Team setup incomplete. Items and members are unavailable until setup is finished.',
      action: 'Resume creation',
    },
    {
      id: 'fed-homelab',
      severity: 'warn',
      title: 'Homelab cannot access Engineering',
      detail:
        'Homelab’s membership in Engineering is inactive. Homelab members cannot access Engineering until it is restored.',
      action: 'Restore access',
    },
  ],
  observedExpiredLeases: [],
  /**
   * An illustrative extension: plaintext representing
   * what `ReadKv` returns after Show. The items themselves stay masked.
   */
  plaintext: {
    'acct:personal|/logins/github.com': 'hx7-Qm2!vTe9-pale-orbit',
    'acct:personal|/logins/fastmail.com': 'kettle-91-Lumen-tide',
    'acct:personal|/env/prod/DATABASE_URL':
      'postgres://service:o4Kq9wLm2x@db.internal/prod',
    'acct:personal|/agents/anthropic-api-key': 'sk-ant-api03-Rf2xQ7bTn41La9mZ4',
    'team:eng|/deploy/production-token': 'foks_team_token_7f31ac09',
    'team:eng|/deploy/staging-token': 'foks_team_token_02be44d1',
    'team:household|/wifi/guest-password': 'sunny-kettle-42',
    'team:household|/streaming/netflix': 'popcorn-Sofa-77',
  },
};

/**
 * The fixture the shell starts from: the fresh lease snapshot, so Engineering
 * and Work list at all.
 */
export const FIXTURE: AgentSnapshot = applyLease(
  {
    ...RAW,
    storeInventory: RAW.stores.map((store) => ({
      store: store.id,
      status: 'available' as const,
      restrictions: [],
    })),
  },
  'fresh',
);

/**
 * Sanctioned desktop wording. Reuse these when the situation matches.
 */
export const COPY = {
  lease_lapsed:
    'This server is locked because its connection expired. Items on this server cannot be viewed or edited until the connection is renewed. Other servers remain available.',
  remove_item:
    'This item will be deleted if it has not been modified by someone else. Earlier versions will no longer be accessible.',
  personal_store_fixed:
    'Your Personal vault is private and cannot be shared. To share an item, move it to a team vault.',
  resumable:
    'Select Resume to continue where you left off. Completed steps will not be repeated.',
} as const;
