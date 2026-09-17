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
  'protect',
  'phrase',
  'waiting',
  'added',
  'create-group',
  'done',
  'local-done',
  'checklist-invited',
  'checklist-own',
] as const;

export type FirstRunStateName = (typeof FIRST_RUN_STATES)[number];
export type FirstRunPath = 'invited' | 'own';
export type FirstRunGroupKind = 'named' | 'adhoc';

export interface FirstRunGroupIdentity {
  readonly name: string;
  readonly kind: FirstRunGroupKind;
  readonly alias: string;
  readonly teamIdHex: string;
}

export interface FirstRunCheckpoint {
  readonly version: 1;
  readonly path: FirstRunPath;
  readonly state: FirstRunStateName;
  readonly initialized: boolean;
  readonly managedLocal: boolean;
  readonly profile?: CheckedProfileResponse;
  readonly serverAddress?: string;
  readonly account?: {
    readonly alias: string;
    readonly username: string;
    readonly deviceName: string;
  };
  readonly passphraseSet: boolean;
  readonly backupCommitted: boolean;
  readonly protectSkipped: boolean;
  readonly group?: FirstRunGroupIdentity;
  readonly groupSkipped: boolean;
  readonly added: boolean;
  readonly returning: boolean;
}

export type FirstRunEvent =
  | { type: 'initialize'; managedLocal?: boolean }
  | { type: 'choose'; path: FirstRunPath; returning?: boolean }
  | {
      type: 'managed-profile-selected';
      address: string;
      profile: CheckedProfileResponse;
      returning?: boolean;
    }
  | { type: 'go'; state: FirstRunStateName }
  | {
      type: 'profile-checked';
      address: string;
      profile: CheckedProfileResponse;
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
  | { type: 'group-complete'; group: FirstRunGroupIdentity }
  | { type: 'skip-group' }
  | { type: 'group-discovered'; group: FirstRunGroupIdentity }
  | { type: 'reenter' };

export function initialFirstRun(
  path: FirstRunPath = 'invited',
  state: FirstRunStateName = 'who',
): FirstRunCheckpoint {
  return {
    version: 1,
    path,
    state,
    initialized: state !== 'boot',
    managedLocal: state === 'local' || state === 'local-done',
    passphraseSet: false,
    backupCommitted: false,
    protectSkipped: false,
    groupSkipped: false,
    added: false,
    returning: false,
  };
}

/** Pure state transition function. */
export function transitionFirstRun(
  state: FirstRunCheckpoint,
  event: FirstRunEvent,
): FirstRunCheckpoint {
  switch (event.type) {
    case 'initialize':
      return {
        ...state,
        path: event.managedLocal ? 'own' : state.path,
        initialized: true,
        managedLocal: event.managedLocal ?? false,
        state: event.managedLocal
          ? 'local'
          : state.state === 'boot'
            ? 'who'
            : state.state,
      };
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
    case 'go':
      return { ...state, state: event.state };
    case 'profile-checked':
      return {
        ...state,
        state: 'checked',
        serverAddress: event.address,
        profile: event.profile,
      };
    case 'account-complete':
      return {
        ...state,
        state: 'protect',
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
    case 'group-complete':
      return {
        ...state,
        state: 'done',
        group: event.group,
        groupSkipped: false,
      };
    case 'skip-group':
      return {
        ...state,
        state: 'checklist-own',
        groupSkipped: state.group === undefined,
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
  'initialized',
  'managedLocal',
  'profile',
  'serverAddress',
  'account',
  'passphraseSet',
  'backupCommitted',
  'protectSkipped',
  'group',
  'groupSkipped',
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
  return state === 'phrase' ? 'protect' : state;
}

/** Serialize safe checkpoint fields to storage, excluding sensitive secrets. */
export function encodeFirstRunCheckpoint(state: FirstRunCheckpoint): string {
  return JSON.stringify({
    version: 1,
    path: state.path,
    state: safeResumeState(state.state),
    initialized: state.initialized,
    managedLocal: state.managedLocal,
    profile: state.profile,
    serverAddress: state.serverAddress,
    account: state.account,
    passphraseSet: state.passphraseSet,
    backupCommitted: state.backupCommitted,
    protectSkipped: state.protectSkipped,
    group: state.group,
    groupSkipped: state.groupSkipped,
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
      item.version !== 1 ||
      !isPath(item.path) ||
      !isFirstRunState(item.state)
    )
      return null;
    const base = initialFirstRun(item.path, safeResumeState(item.state));
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
    const flags = [
      item.initialized,
      item.managedLocal,
      item.passphraseSet,
      item.backupCommitted,
      item.protectSkipped,
      item.groupSkipped,
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
    const needsProfile = ![
      'boot',
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
      'create-group',
      'done',
      'local-done',
      'checklist-invited',
      'checklist-own',
    ].includes(state);
    if ((state === 'boot') === (item.initialized as boolean)) return null;
    if (profile && item.serverAddress === undefined) return null;
    if (needsProfile && (!profile || item.serverAddress === undefined))
      return null;
    if (account && !profile) return null;
    if (needsAccount && !account) return null;
    if (
      (item.passphraseSet === true ||
        item.backupCommitted === true ||
        item.protectSkipped === true ||
        group ||
        item.groupSkipped === true ||
        item.added === true) &&
      !account
    )
      return null;
    if (group && !account) return null;
    if (
      item.protectSkipped === true &&
      (item.passphraseSet === true || item.backupCommitted === true)
    )
      return null;
    if (item.groupSkipped === true && (group || item.added === true))
      return null;
    if (item.returning === true && item.path !== 'own') return null;
    if (item.managedLocal === true && item.path !== 'own') return null;
    if (
      (state === 'local' || state === 'local-done') &&
      item.managedLocal !== true
    )
      return null;
    if (state === 'done' && (!group || item.path !== 'own')) return null;
    if (
      state === 'added' &&
      (item.added !== true || item.path !== 'invited' || !group)
    )
      return null;
    if (item.added === true && item.path !== 'invited') return null;
    if (item.path === 'invited' && Boolean(group) !== (item.added === true))
      return null;
    if (item.path === 'own' && item.added === true) return null;
    if (item.groupSkipped === true && item.path !== 'own') return null;
    return {
      ...base,
      initialized: item.initialized as boolean,
      managedLocal: item.managedLocal as boolean,
      profile,
      serverAddress:
        typeof item.serverAddress === 'string' ? item.serverAddress : undefined,
      account,
      passphraseSet: item.passphraseSet as boolean,
      backupCommitted: item.backupCommitted as boolean,
      protectSkipped: item.protectSkipped as boolean,
      group,
      groupSkipped: item.groupSkipped as boolean,
      added: item.added as boolean,
      returning: item.returning as boolean,
    };
  } catch {
    return null;
  }
}

export const FIRST_RUN_CHECKPOINT_KEY = 'foks.first-run.v1';

export function completedFirstRunSteps(state: FirstRunCheckpoint): number {
  let count = state.initialized ? 1 : 0;
  if (state.profile) count += 1;
  if (state.account) count += 1;
  // Skipped safeguards or groups do not count toward completed steps.
  if (state.passphraseSet || state.backupCommitted) count += 1;
  if (state.added || state.group) count += 1;
  return count;
}
