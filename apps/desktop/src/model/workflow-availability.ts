import { serverFactAvailability } from './lease';
import type { AvailabilityReason } from './lease';
import type { AgentSnapshot, ProtocolCapability, ServerFailure } from './types';

export const WORKFLOW_REQUIREMENTS = {
  'local-alias': [],
  'account-sync': ['user-sync', 'kv'],
  'account-rename': ['device-administration'],
  'devices-list': ['device-administration'],
  'device-remove': ['device-administration'],
  'device-pair': ['device-administration'],
  'device-accept': ['device-administration'],
  'backup-list': [],
  'backup-create': ['recovery'],
  'backup-revoke': ['recovery', 'device-administration'],
  'account-recover': ['recovery'],
  passphrase: ['passphrases'],
  'sso-login': ['user-sync'],
  'sso-signup': ['signup'],
  'sso-yubi-signup': ['signup', 'device-administration'],
  'bot-list': [],
  'bot-unload': [],
  'bot-load': ['device-administration'],
  'bot-enroll': ['device-administration'],
  'bot-revoke': ['device-administration'],
  'web-admin-configure': [],
  'web-admin-open': ['user-sync'],
  'yubi-list': [],
  'yubi-scan': ['device-administration'],
  'yubi-create': ['signup', 'device-administration'],
  'yubi-resume': ['device-administration'],
  'yubi-provision': ['device-administration'],
  'yubi-revoke': ['device-administration'],
  'yubi-pin': ['device-administration'],
  'yubi-rotate': ['device-administration'],
  'yubi-recover-management': ['device-administration', 'recovery'],
  'yubi-recover-subkey': ['recovery'],
  federate: ['teams', 'federation'],
} as const satisfies Record<string, readonly ProtocolCapability[]>;

export type WorkflowOperation = keyof typeof WORKFLOW_REQUIREMENTS;
export type WorkflowAvailability =
  | { available: true }
  | {
      available: false;
      reason:
        | AvailabilityReason
        | 'credentials-needed'
        | 'hardware-needed'
        | 'auth-needed'
        | 'permission-denied'
        | 'unreachable'
        | 'unknown';
      capability?: ProtocolCapability;
    };

export interface WorkflowTarget {
  store?: string;
  profile?: string;
  account?: string;
  remoteProfile?: string;
  reachability?: {
    operation: WorkflowOperation;
    status: 'reachable' | 'unreachable';
    observedAt: number;
  };
  credentials?: 'ready' | 'needed' | 'unknown';
  hardware?: 'ready' | 'needed' | 'unknown';
  authentication?: 'ready' | 'needed' | 'unknown';
  permission?: 'allowed' | 'denied' | 'unknown';
  passphraseRequired?: boolean;
  signupRequired?: boolean;
  nowSeconds?: number;
}

const localOperations = new Set<WorkflowOperation>([
  'local-alias',
  'bot-list',
  'bot-unload',
  'web-admin-configure',
  'yubi-list',
  'backup-list',
]);

function accountFailure(
  error?: ServerFailure,
): WorkflowAvailability | undefined {
  switch (error?.code) {
    case 'bot-token-locked':
    case 'credentials-required':
      return { available: false, reason: 'credentials-needed' };
    case 'reauthentication-required':
      return { available: false, reason: 'auth-needed' };
    default:
      return undefined;
  }
}

export function workflowAvailability(
  snapshot: AgentSnapshot | undefined,
  operation: WorkflowOperation,
  target: WorkflowTarget,
): WorkflowAvailability {
  if (!snapshot) return { available: false, reason: 'unknown' };
  if (snapshot.agent.state !== 'ready')
    return { available: false, reason: 'agent-unavailable' };
  if (target.store) {
    const store = snapshot.stores.find((entry) => entry.id === target.store);
    if (
      !store ||
      (target.profile && store.server !== target.profile) ||
      (target.account && store.account !== target.account)
    )
      return { available: false, reason: 'unknown' };
    target = { ...target, profile: store.server, account: store.account };
  }
  const server = snapshot.servers.find(
    (entry) => entry.profileName === target.profile,
  );
  if (!server) return { available: false, reason: 'unknown' };
  if (
    operation === 'local-alias' &&
    !snapshot.accounts.some(
      (account) =>
        account.server === target.profile && account.alias === target.account,
    ) &&
    !snapshot.stores.some(
      (store) =>
        store.kind === 'account' &&
        store.server === target.profile &&
        store.account === target.account,
    )
  )
    return { available: false, reason: 'unknown' };
  if (localOperations.has(operation)) return { available: true };
  const capabilities: ProtocolCapability[] = [
    ...WORKFLOW_REQUIREMENTS[operation],
  ];
  if (target.passphraseRequired) capabilities.push('passphrases');
  if (target.signupRequired) capabilities.push('signup');
  const serverAccess = serverFactAvailability(
    server,
    snapshot.observedExpiredLeases,
    { nowSeconds: target.nowSeconds },
    capabilities,
  );
  if (!serverAccess.available)
    return serverAccess.reason === 'server-status-unavailable' &&
      server.passiveStatus.status === 'failed' &&
      server.passiveStatus.error.code === 'credentials-required'
      ? { available: false, reason: 'credentials-needed' }
      : serverAccess;
  const reachability = target.reachability;
  const elapsed = reachability
    ? (target.nowSeconds ?? Date.now() / 1_000) - reachability.observedAt
    : Infinity;
  if (
    reachability?.operation === operation &&
    reachability.status === 'unreachable' &&
    elapsed >= 0 &&
    elapsed < 30
  )
    return { available: false, reason: 'unreachable' };
  if (operation === 'federate') {
    if (!target.remoteProfile) return { available: false, reason: 'unknown' };
    const remoteServer = snapshot.servers.find(
      (entry) => entry.profileName === target.remoteProfile,
    );
    if (!remoteServer) return { available: false, reason: 'unknown' };
    const access = serverFactAvailability(
      remoteServer,
      snapshot.observedExpiredLeases,
      { nowSeconds: target.nowSeconds },
      capabilities,
    );
    if (!access.available) return access;
  }
  const account = snapshot.accounts.find(
    (entry) =>
      entry.server === target.profile && entry.alias === target.account,
  );
  const inventory =
    account &&
    snapshot.storeInventory.find((entry) => entry.store === account.store);
  for (const restriction of inventory?.restrictions ?? []) {
    if (
      restriction.kind === 'capability-denied' &&
      capabilities.includes(restriction.capability)
    )
      return {
        available: false,
        reason: 'capability-unavailable',
        capability: restriction.capability,
      };
    if (
      restriction.kind === 'schema-incompatible' ||
      restriction.kind === 'import-verification-required'
    )
      return { available: false, reason: restriction.kind };
  }
  const failure = accountFailure(inventory?.error);
  if (
    failure &&
    !failure.available &&
    !(operation === 'sso-login' && failure.reason === 'auth-needed') &&
    operation !== 'bot-load' &&
    operation !== 'account-recover'
  )
    return failure;
  if (target.credentials === 'needed')
    return { available: false, reason: 'credentials-needed' };
  if (target.hardware === 'needed')
    return { available: false, reason: 'hardware-needed' };
  if (target.authentication === 'needed' && operation !== 'sso-login')
    return { available: false, reason: 'auth-needed' };
  if (target.permission === 'denied')
    return { available: false, reason: 'permission-denied' };
  if (
    [
      target.credentials,
      target.hardware,
      target.authentication,
      target.permission,
    ].includes('unknown')
  )
    return { available: false, reason: 'unknown' };
  return { available: true };
}

export function workflowMessage(
  access: WorkflowAvailability,
): string | undefined {
  if (access.available) return undefined;
  switch (access.reason) {
    case 'credentials-needed':
      return 'Load this account’s credentials to continue.';
    case 'hardware-needed':
      return 'Connect and explicitly unlock the required security key.';
    case 'auth-needed':
      return 'Sign in to this account to continue.';
    case 'permission-denied':
      return 'This account is not permitted to perform this operation.';
    case 'capability-unavailable':
      return `The server does not permit ${access.capability ?? 'this operation'}.`;
    case 'unreachable':
      return 'The server could not be reached.';
    case 'compatibility-incompatible':
      return 'The server protocol is incompatible.';
    case 'check-in-expired':
      return 'The server check-in has expired.';
    case 'check-in-unavailable':
      return 'The server check-in is unavailable.';
    case 'verification-failed':
      return 'Server verification failed.';
    case 'security-state-missing':
      return 'Restore saved security state before continuing.';
    case 'verification-required':
    case 'import-verification-required':
      return 'Server verification is required.';
    case 'schema-incompatible':
      return 'The local schema is incompatible.';
    case 'agent-unavailable':
      return 'The background service is unavailable.';
    default:
      return 'Operation readiness is not known. Refresh status before continuing.';
  }
}

export function requireWorkflow(
  snapshot: AgentSnapshot | undefined,
  operation: WorkflowOperation,
  target: WorkflowTarget,
): void {
  const access = workflowAvailability(snapshot, operation, target);
  if (!access.available)
    throw Object.assign(new Error(workflowMessage(access)), {
      code: 'workflow-unavailable',
      retryable: false,
      fatal: false,
      ambiguous: false,
      details: {
        reason: access.reason,
        ...(access.capability ? { capability: access.capability } : {}),
      },
    });
}
