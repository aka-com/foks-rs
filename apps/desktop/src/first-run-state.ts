import type { CheckedProfileResponse } from './bridge';

export const FIRST_RUN_STATES = [
  'boot',
  'who',
  'local',
  'address',
  'no-address',
  'checked',
  'compare',
  'error',
  'account',
  'existing',
  'identity-pending',
  'operation-pending',
  'protect',
  'phrase',
  'waiting',
  'added',
  'local-done',
  'checklist-invited',
  'checklist-own',
] as const;

export type FirstRunStateName = (typeof FIRST_RUN_STATES)[number];
export type FirstRunPath = 'invited' | 'own';
export type FirstRunGroupKind = 'named' | 'adhoc';
export type FirstRunAccountMethod = 'create' | 'recover' | 'organization';

export interface FirstRunGroupIdentity {
  readonly name: string;
  readonly kind: FirstRunGroupKind;
  readonly alias: string;
  readonly teamIdHex: string;
}

export interface FirstRunCheckpoint {
  readonly version: 3;
  readonly path: FirstRunPath;
  readonly state: FirstRunStateName;
  readonly managedLocal: boolean;
  /** Account methods share one step and one deterministic Back destination. */
  readonly accountMethod?: FirstRunAccountMethod;
  readonly profile?: CheckedProfileResponse;
  readonly serverAddress?: string;
  /** Nonsecret intent retained until a command acknowledges its outcome. */
  readonly provisioning?: ProvisioningIntent;
  readonly sso?: {
    readonly operationId: string;
    readonly alias: string;
    readonly hardware: boolean;
  };
  /** Acknowledged provisioning; never replay it while identity is loading. */
  readonly provisionedAccount?: {
    readonly alias: string;
    readonly deviceName: string;
  };
  readonly account?: {
    readonly alias: string;
    readonly username: string;
    readonly deviceName: string;
  };
  readonly passphraseSet: boolean;
  readonly backupCommitted: boolean;
  readonly protectSkipped: boolean;
  readonly group?: FirstRunGroupIdentity;
  readonly selectedGroup?: FirstRunGroupIdentity;
  readonly added: boolean;
  readonly returning: boolean;
}

export interface ProvisioningIntent {
  readonly id: string;
  readonly kind: 'signup' | 'recovery' | 'copy' | 'pairing' | 'sso';
  readonly alias: string;
  readonly deviceName: string;
  readonly back: 'account' | 'existing';
  readonly candidateId?: string;
  readonly ssoOperationId?: string;
}

export type FirstRunEvent =
  | { type: 'choose'; path: FirstRunPath; returning?: boolean }
  | {
      type: 'managed-profile-selected';
      address: string;
      profile: CheckedProfileResponse;
      returning?: boolean;
    }
  | { type: 'navigate'; state: FirstRunStateName }
  | { type: 'back' }
  | { type: 'select-account-method'; method: FirstRunAccountMethod }
  | { type: 'server-edited'; address: string }
  | {
      type: 'profile-checked';
      address: string;
      profile: CheckedProfileResponse;
    }
  | {
      type: 'account-provisioned';
      alias: string;
      deviceName: string;
    }
  | {
      type: 'account-complete';
      alias: string;
      username: string;
      deviceName: string;
    }
  | { type: 'passphrase-set' }
  | { type: 'backup-committed' }
  | { type: 'skip-protect' }
  | { type: 'finish-local'; skipped: boolean }
  | { type: 'group-discovered'; group: FirstRunGroupIdentity }
  | { type: 'discard-provisioning' }
  | { type: 'discard-provisioned-account' }
  | { type: 'reenter' };

export function initialFirstRun(
  path: FirstRunPath = 'invited',
  state: FirstRunStateName = 'who',
): FirstRunCheckpoint {
  return {
    version: 3,
    path,
    state,
    managedLocal: state === 'local' || state === 'local-done',
    passphraseSet: false,
    backupCommitted: false,
    protectSkipped: false,
    added: false,
    returning: false,
  };
}

/** Runtime-validated state used by the controller; the disk codec accepts the broad input shape. */
export type ResolvedFirstRunCheckpoint = FirstRunCheckpoint &
  (
    | { state: 'boot' | 'who' | 'local' | 'address' | 'no-address' | 'error' }
    | {
        state: 'checked' | 'compare' | 'account' | 'existing';
        profile: CheckedProfileResponse;
      }
    | {
        state: 'operation-pending';
        profile: CheckedProfileResponse;
        provisioning: ProvisioningIntent;
      }
    | {
        state: 'identity-pending';
        profile: CheckedProfileResponse;
        provisionedAccount: NonNullable<
          FirstRunCheckpoint['provisionedAccount']
        >;
      }
    | {
        state:
          | 'protect'
          | 'phrase'
          | 'waiting'
          | 'added'
          | 'local-done'
          | 'checklist-invited'
          | 'checklist-own';
        profile: CheckedProfileResponse;
        account: NonNullable<FirstRunCheckpoint['account']>;
      }
  );

export function normalizeFirstRunCheckpoint(
  saved: FirstRunCheckpoint,
): ResolvedFirstRunCheckpoint {
  // Only this boundary promotes a decoded/draft checkpoint to a renderable state.
  return resolvePrerequisites(saved) as ResolvedFirstRunCheckpoint;
}

/** Resolve incomplete entry requests without inventing server/account evidence. */
function resolvePrerequisites(saved: FirstRunCheckpoint): FirstRunCheckpoint {
  if ((saved.provisioning || saved.provisionedAccount) && !saved.profile)
    throw new Error('Account setup is missing its server identity.');
  if (saved.provisioning)
    return saved.state === 'operation-pending'
      ? saved
      : { ...saved, state: 'operation-pending' };
  if (saved.provisionedAccount)
    return saved.state === 'identity-pending'
      ? saved
      : { ...saved, state: 'identity-pending' };
  const accountStates: readonly FirstRunStateName[] = [
    'protect',
    'phrase',
    'waiting',
    'added',
    'local-done',
    'checklist-invited',
    'checklist-own',
  ];
  const profileStates: readonly FirstRunStateName[] = [
    'checked',
    'compare',
    'account',
    'existing',
    ...accountStates,
  ];
  if (profileStates.includes(saved.state) && !saved.profile)
    return {
      ...initialFirstRun(saved.path, saved.serverAddress ? 'address' : 'who'),
      serverAddress: saved.serverAddress,
    };
  if (accountStates.includes(saved.state) && !saved.account)
    return {
      ...saved,
      state: saved.returning ? 'existing' : 'account',
      passphraseSet: false,
      backupCommitted: false,
      protectSkipped: false,
      group: undefined,
      selectedGroup: undefined,
      added: false,
    };
  if (saved.state === 'identity-pending' || saved.state === 'operation-pending')
    return { ...saved, state: saved.profile ? 'account' : 'who' };
  return saved;
}

export function setupBackTarget(state: FirstRunCheckpoint): FirstRunStateName {
  switch (state.state) {
    case 'account':
    case 'existing':
      return state.managedLocal ? 'local' : 'checked';
    case 'protect':
    case 'phrase':
      return state.returning ? 'existing' : 'account';
    case 'checked':
    case 'compare':
      return 'address';
    default:
      return 'who';
  }
}

/** Pure state transition function. */
export function transitionFirstRun(
  state: FirstRunCheckpoint,
  event: FirstRunEvent,
): FirstRunCheckpoint {
  if (event.type === 'discard-provisioning') {
    if (!state.provisioning) return state;
    return {
      ...state,
      state: state.provisioning.back,
      provisioning: undefined,
      sso: undefined,
    };
  }
  if (event.type === 'discard-provisioned-account') {
    if (!state.provisionedAccount) return state;
    return {
      ...state,
      state: state.returning ? 'existing' : 'account',
      provisionedAccount: undefined,
      sso: undefined,
    };
  }
  if (state.provisioning) return state;
  if (
    state.provisionedAccount &&
    (event.type !== 'account-complete' ||
      event.alias !== state.provisionedAccount.alias)
  )
    return state;
  switch (event.type) {
    case 'choose':
      return {
        ...initialFirstRun(event.path, 'address'),
        returning: event.returning ?? false,
      };
    case 'managed-profile-selected':
      return {
        ...initialFirstRun('own', event.returning ? 'existing' : 'account'),
        managedLocal: true,
        serverAddress: event.address,
        profile: event.profile,
        returning: event.returning ?? false,
      };
    case 'back':
      return transitionFirstRun(state, {
        type: 'navigate',
        state: setupBackTarget(state),
      });
    case 'select-account-method':
      return normalizeFirstRunCheckpoint({
        ...state,
        state: event.method === 'recover' ? 'existing' : 'account',
        accountMethod: event.method,
      });
    case 'navigate': {
      const next = normalizeFirstRunCheckpoint({
        ...state,
        state: event.state,
      });
      // A phrase reveal is allowed only as an explicit live transition.
      return event.state === 'phrase' && state.profile && state.account
        ? { ...next, state: 'phrase' }
        : next;
    }
    case 'server-edited':
      return {
        ...initialFirstRun(state.path, 'address'),
        returning: state.returning,
        serverAddress: event.address,
      };
    case 'profile-checked':
      return {
        ...state,
        state: 'checked',
        serverAddress: event.address,
        profile: event.profile,
      };
    case 'account-provisioned':
      return {
        ...state,
        state: 'identity-pending',
        provisionedAccount: {
          alias: event.alias,
          deviceName: event.deviceName,
        },
      };
    case 'account-complete':
      return {
        ...state,
        state: 'protect',
        sso: undefined,
        provisionedAccount: undefined,
        account: {
          alias: event.alias,
          username: event.username,
          deviceName: event.deviceName,
        },
      };
    case 'passphrase-set':
      // Setting a passphrase clears protectSkipped so the checkpoint remains valid.
      return { ...state, passphraseSet: true, protectSkipped: false };
    case 'backup-committed':
      return {
        ...state,
        state: 'protect',
        backupCommitted: true,
        protectSkipped: false,
      };
    case 'skip-protect':
      return {
        ...state,
        state: state.path === 'invited' ? 'checklist-invited' : 'checklist-own',
        protectSkipped: !state.passphraseSet && !state.backupCommitted,
      };
    case 'finish-local':
      return {
        ...state,
        state: 'local-done',
        protectSkipped:
          event.skipped && !state.passphraseSet && !state.backupCommitted,
      };
    case 'group-discovered':
      return { ...state, state: 'added', added: true, group: event.group };
    case 'reenter':
      return initialFirstRun(state.path, 'who');
  }
}

const isPath = (value: unknown): value is FirstRunPath =>
  value === 'invited' || value === 'own';
export const isFirstRunState = (value: unknown): value is FirstRunStateName =>
  typeof value === 'string' &&
  (FIRST_RUN_STATES as readonly string[]).includes(value);

const ROOT_KEYS = new Set([
  'version',
  'path',
  'state',
  'managedLocal',
  'accountMethod',
  'profile',
  'serverAddress',
  'provisioning',
  'sso',
  'account',
  'provisionedAccount',
  'passphraseSet',
  'backupCommitted',
  'protectSkipped',
  'group',
  'selectedGroup',
  'added',
  'returning',
]);
const PROFILE_KEYS = new Set([
  'profile',
  'acceptance',
  'lookupName',
  'canonicalName',
  'hostId',
  'chain',
  'epoch',
]);
const ACCOUNT_KEYS = new Set(['alias', 'username', 'deviceName']);
const PROVISIONED_ACCOUNT_KEYS = new Set(['alias', 'deviceName']);
const GROUP_KEYS = new Set(['name', 'kind', 'alias', 'teamIdHex']);

function exactKeys(
  value: Record<string, unknown>,
  allowed: ReadonlySet<string>,
): boolean {
  return Object.keys(value).every((key) => allowed.has(key));
}

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function boundedText(value: unknown, maximum: number): value is string {
  return (
    typeof value === 'string' &&
    value.length > 0 &&
    value.length <= maximum &&
    value.trim() === value &&
    [...value].every((character) => {
      const code = character.charCodeAt(0);
      return code > 31 && code !== 127;
    })
  );
}

function localName(value: unknown): value is string {
  return typeof value === 'string' && /^[A-Za-z0-9_-]{1,64}$/.test(value);
}

function safeResumeState(state: FirstRunStateName): FirstRunStateName {
  // A recovery phrase is ephemeral; resuming returns to 'protect' to require a new reveal.
  if (state === 'phrase') return 'protect';
  // Bootstrap is authoritative agent state, never persisted onboarding progress.
  return state === 'boot' ? 'who' : state;
}

/** Serialize safe checkpoint fields to storage, excluding sensitive secrets. */
export function encodeFirstRunCheckpoint(state: FirstRunCheckpoint): string {
  return JSON.stringify({
    version: 3,
    path: state.path,
    state: safeResumeState(state.state),
    managedLocal: state.managedLocal,
    accountMethod: state.accountMethod,
    profile: state.profile,
    serverAddress: state.serverAddress,
    account: state.account,
    provisionedAccount: state.provisionedAccount,
    provisioning: state.provisioning,
    sso: state.sso,
    passphraseSet: state.passphraseSet,
    backupCommitted: state.backupCommitted,
    protectSkipped: state.protectSkipped,
    group: state.group,
    selectedGroup: state.selectedGroup,
    added: state.added,
    returning: state.returning,
  });
}

export function decodeFirstRunCheckpoint(
  value: string | null,
): FirstRunCheckpoint | null {
  if (!value) return null;
  try {
    const parsed: unknown = JSON.parse(value);
    if (!record(parsed) || !exactKeys(parsed, ROOT_KEYS)) return null;
    const item = parsed;
    if (
      (item.version !== 3 && item.version !== 2) ||
      !isPath(item.path) ||
      !isFirstRunState(item.state) ||
      item.state === 'boot'
    )
      return null;
    const base = initialFirstRun(item.path, safeResumeState(item.state));
    if (
      item.accountMethod !== undefined &&
      !['create', 'recover', 'organization'].includes(
        String(item.accountMethod),
      )
    )
      return null;
    let sso: FirstRunCheckpoint['sso'];
    if (item.sso !== undefined) {
      const value = item.sso;
      if (
        item.version !== 3 ||
        !record(value) ||
        !exactKeys(value, new Set(['operationId', 'alias', 'hardware'])) ||
        typeof value.operationId !== 'string' ||
        !/^[a-f0-9]{32}$/.test(value.operationId) ||
        !localName(value.alias) ||
        typeof value.hardware !== 'boolean'
      )
        return null;
      sso = {
        operationId: value.operationId,
        alias: value.alias,
        hardware: value.hardware,
      };
    }
    let provisioning: ProvisioningIntent | undefined;
    if (item.provisioning !== undefined) {
      const candidate = item.provisioning;
      if (
        item.version !== 3 ||
        !record(candidate) ||
        !exactKeys(
          candidate,
          new Set([
            'id',
            'kind',
            'alias',
            'deviceName',
            'back',
            'candidateId',
            'ssoOperationId',
          ]),
        ) ||
        !boundedText(candidate.id, 64) ||
        !localName(candidate.alias) ||
        !boundedText(candidate.deviceName, 256) ||
        !['signup', 'recovery', 'copy', 'pairing', 'sso'].includes(
          String(candidate.kind),
        ) ||
        !['account', 'existing'].includes(String(candidate.back)) ||
        (candidate.candidateId !== undefined &&
          !boundedText(candidate.candidateId, 512)) ||
        (candidate.ssoOperationId !== undefined &&
          (typeof candidate.ssoOperationId !== 'string' ||
            !/^[a-f0-9]{32}$/.test(candidate.ssoOperationId)))
      )
        return null;
      provisioning = candidate as unknown as ProvisioningIntent;
    }
    let profile: CheckedProfileResponse | undefined;
    if (item.profile !== undefined) {
      const candidate = item.profile;
      if (
        !record(candidate) ||
        !exactKeys(candidate, PROFILE_KEYS) ||
        (candidate.acceptance !== 'inserted' &&
          candidate.acceptance !== 'advanced' &&
          candidate.acceptance !== 'unchanged') ||
        !localName(candidate.profile) ||
        !boundedText(candidate.lookupName, 256) ||
        !boundedText(candidate.canonicalName, 256) ||
        typeof candidate.hostId !== 'string' ||
        !/^02[0-9a-f]{64}$/.test(candidate.hostId) ||
        typeof candidate.chain !== 'number' ||
        !Number.isSafeInteger(candidate.chain) ||
        candidate.chain < 0 ||
        typeof candidate.epoch !== 'number' ||
        !Number.isSafeInteger(candidate.epoch) ||
        candidate.epoch < 0
      )
        return null;
      profile = {
        profile: candidate.profile,
        acceptance: candidate.acceptance,
        lookupName: candidate.lookupName,
        canonicalName: candidate.canonicalName,
        hostId: candidate.hostId,
        chain: candidate.chain,
        epoch: candidate.epoch,
      };
    }
    let account: FirstRunCheckpoint['account'];
    if (item.account !== undefined) {
      const candidate = item.account;
      if (
        !record(candidate) ||
        !exactKeys(candidate, ACCOUNT_KEYS) ||
        !localName(candidate.alias) ||
        !boundedText(candidate.username, 256) ||
        !boundedText(candidate.deviceName, 256)
      )
        return null;
      account = {
        alias: candidate.alias,
        username: candidate.username,
        deviceName: candidate.deviceName,
      };
    }
    let provisionedAccount: FirstRunCheckpoint['provisionedAccount'];
    if (item.provisionedAccount !== undefined) {
      const candidate = item.provisionedAccount;
      if (
        !record(candidate) ||
        !exactKeys(candidate, PROVISIONED_ACCOUNT_KEYS) ||
        !localName(candidate.alias) ||
        !boundedText(candidate.deviceName, 256)
      )
        return null;
      provisionedAccount = {
        alias: candidate.alias,
        deviceName: candidate.deviceName,
      };
    }
    let group: FirstRunCheckpoint['group'];
    if (item.group !== undefined) {
      const candidate = item.group;
      if (
        !record(candidate) ||
        !exactKeys(candidate, GROUP_KEYS) ||
        !boundedText(candidate.name, 256) ||
        (candidate.kind !== 'named' && candidate.kind !== 'adhoc') ||
        !localName(candidate.alias) ||
        typeof candidate.teamIdHex !== 'string' ||
        !new RegExp(
          `^${candidate.kind === 'named' ? '03' : '14'}[0-9a-f]{64}$`,
        ).test(candidate.teamIdHex)
      )
        return null;
      group = {
        name: candidate.name,
        kind: candidate.kind,
        alias: candidate.alias,
        teamIdHex: candidate.teamIdHex,
      };
    }
    let selectedGroup: FirstRunGroupIdentity | undefined;
    if (item.selectedGroup !== undefined) {
      const candidate = item.selectedGroup;
      if (
        item.version !== 3 ||
        !record(candidate) ||
        !exactKeys(candidate, GROUP_KEYS) ||
        !boundedText(candidate.name, 256) ||
        !localName(candidate.alias) ||
        (candidate.kind !== 'named' && candidate.kind !== 'adhoc') ||
        typeof candidate.teamIdHex !== 'string' ||
        !new RegExp(
          `^${candidate.kind === 'named' ? '03' : '14'}[0-9a-f]{64}$`,
        ).test(candidate.teamIdHex)
      )
        return null;
      selectedGroup = {
        name: candidate.name,
        alias: candidate.alias,
        kind: candidate.kind,
        teamIdHex: candidate.teamIdHex,
      };
    }
    const flags = [
      item.managedLocal,
      item.passphraseSet,
      item.backupCommitted,
      item.protectSkipped,
      item.added,
      item.returning,
    ];
    if (flags.some((flag) => typeof flag !== 'boolean')) return null;
    if (
      item.serverAddress !== undefined &&
      !boundedText(item.serverAddress, 2048)
    )
      return null;
    const state = safeResumeState(item.state);
    if (Boolean(provisioning) !== (state === 'operation-pending')) return null;
    if (provisioning && (!profile || account || provisionedAccount))
      return null;
    if (Boolean(provisionedAccount) !== (state === 'identity-pending'))
      return null;
    if (provisionedAccount && (!profile || account)) return null;
    const needsProfile = ![
      'who',
      'local',
      'address',
      'no-address',
      'error',
    ].includes(state);
    const needsAccount = [
      'protect',
      'waiting',
      'added',
      'local-done',
      'checklist-invited',
      'checklist-own',
    ].includes(state);
    if (profile && item.serverAddress === undefined) return null;
    if (needsProfile && (!profile || item.serverAddress === undefined))
      return null;
    if (account && !profile) return null;
    if (sso && !profile) return null;
    if (needsAccount && !account) return null;
    if (
      (item.passphraseSet === true ||
        item.backupCommitted === true ||
        item.protectSkipped === true ||
        group ||
        item.added === true) &&
      !account
    )
      return null;
    if (group && !account) return null;
    if (selectedGroup && (!account || item.path !== 'invited')) return null;
    if (
      item.protectSkipped === true &&
      (item.passphraseSet === true || item.backupCommitted === true)
    )
      return null;
    if (item.returning === true && item.path !== 'own') return null;
    if (item.managedLocal === true && item.path !== 'own') return null;
    if (
      (state === 'local' || state === 'local-done') &&
      item.managedLocal !== true
    )
      return null;
    if (
      state === 'added' &&
      (item.added !== true || item.path !== 'invited' || !group)
    )
      return null;
    if (item.added === true && item.path !== 'invited') return null;
    if (item.path === 'invited' && Boolean(group) !== (item.added === true))
      return null;
    if (item.path === 'own' && (group || item.added === true)) return null;
    return {
      ...base,
      managedLocal: item.managedLocal as boolean,
      ...(item.accountMethod
        ? { accountMethod: item.accountMethod as FirstRunAccountMethod }
        : {}),
      profile,
      serverAddress:
        typeof item.serverAddress === 'string' ? item.serverAddress : undefined,
      account,
      provisionedAccount,
      ...(provisioning ? { provisioning } : {}),
      ...(sso ? { sso } : {}),
      passphraseSet: item.passphraseSet as boolean,
      backupCommitted: item.backupCommitted as boolean,
      protectSkipped: item.protectSkipped as boolean,
      group,
      ...(selectedGroup ? { selectedGroup } : {}),
      added: item.added as boolean,
      returning: item.returning as boolean,
    };
  } catch {
    return null;
  }
}

export const FIRST_RUN_CHECKPOINT_KEY = 'foks.first-run.v2';

export type SetupFact = 'present' | 'missing' | 'unknown';

export interface FirstRunSetupFacts {
  readonly profile: SetupFact;
  readonly account: SetupFact;
  readonly group: SetupFact;
}

/**
 * Reconcile saved UI progress with authoritative inventory. Unknown facts are
 * deliberately non-destructive: an incomplete catalog is not evidence that
 * local setup data disappeared.
 */
export function reconcileFirstRunCheckpoint(
  saved: FirstRunCheckpoint,
  facts: FirstRunSetupFacts,
): FirstRunCheckpoint {
  // Missing or incomplete inventory cannot undo an acknowledged mutation.
  // Identity resolution checks the saved host/profile binding separately.
  if (saved.provisionedAccount) return saved;
  if (saved.provisioning) return saved;
  if (saved.profile && facts.profile === 'missing') {
    return {
      ...initialFirstRun(saved.path, 'address'),
      serverAddress: saved.serverAddress,
      returning: saved.returning,
    };
  }
  if (saved.account && facts.account === 'missing') {
    return {
      ...initialFirstRun(saved.path, 'account'),
      managedLocal: saved.managedLocal,
      profile: saved.profile,
      serverAddress: saved.serverAddress,
      returning: saved.returning,
    };
  }
  if (
    saved.path === 'invited' &&
    saved.added &&
    saved.group &&
    facts.group === 'missing'
  ) {
    return {
      ...saved,
      state: 'waiting',
      group: undefined,
      selectedGroup: saved.group,
      added: false,
    };
  }
  return saved;
}

export function completedFirstRunSteps(state: FirstRunCheckpoint): number {
  let count = 0;
  if (state.profile) count += 1;
  if (state.account) count += 1;
  // Skipped recovery does not count toward completed steps.
  if (state.passphraseSet || state.backupCommitted) count += 1;
  if (state.path === 'invited' && state.added) count += 1;
  return count;
}

/**
 * One line naming the step setup wants next, for the rail's setup card. The
 * order follows `completedFirstRunSteps`: a server, an account, recovery, and
 * for an invitee the group.
 */
export function firstRunNextStep(state: FirstRunCheckpoint): string {
  if (!state.profile) return 'choose a server';
  if (!state.account) return 'create or recover your account';
  if (!(state.passphraseSet || state.backupCommitted))
    return 'save your recovery codes';
  if (state.path === 'invited' && !state.added) return 'join your team';
  return 'finish setup';
}

/** Returns the setup screen that performs the next incomplete step. */
export function firstRunNextStepState(
  state: FirstRunCheckpoint,
): FirstRunStateName {
  if (!state.profile) return 'who';
  if (!state.account) return 'account';
  if (!(state.passphraseSet || state.backupCommitted)) return 'protect';
  if (state.path === 'invited' && !state.added) return 'waiting';
  return state.state;
}

/** Personal setup ends after account recovery; invitees also join their group. */
export function firstRunStepCount(state: FirstRunCheckpoint): number {
  return state.path === 'invited' ? 4 : 3;
}
