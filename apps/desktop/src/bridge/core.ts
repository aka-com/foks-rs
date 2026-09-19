import type { AgentStatus } from '../model';
import { decodeCommandError, type CommandError } from './errors';
import {
  array,
  bool,
  integer,
  nullableInteger,
  nullableString,
  optionalString,
  record,
  string,
} from './validation';

export type MaintenanceKind =
  'export' | 'import' | 'verify' | 'relocate' | 'restart';
export type MaintenancePhase =
  'selecting' | 'confirming' | 'quiescing' | 'running' | 'restoring';
export type MaintenanceOperationOutcome =
  | { status: 'cancelled' }
  | { status: 'completed' }
  | { status: 'failed'; error: CommandError };
export type MaintenanceDisposition =
  | { status: 'continue-current-root' }
  | { status: 'restart-selected-root'; root: string }
  | { status: 'recovery-required'; root: string }
  | { status: 'restoration-failed'; root: string; error: CommandError };
export type MaintenanceSnapshot =
  | { state: 'idle'; generation: number; revision: number }
  | {
      state: 'active';
      generation: number;
      revision: number;
      kind: MaintenanceKind;
      phase: MaintenancePhase;
    }
  | {
      state: 'complete';
      generation: number;
      revision: number;
      kind: MaintenanceKind;
      operation: MaintenanceOperationOutcome;
      disposition: MaintenanceDisposition;
    };

export interface DropHoverEvent {
  hovering: boolean;
}

export interface WindowStateEvent {
  maximized: boolean;
  fullscreen: boolean;
}

/** The process answering on the agent socket, as Settings › Device shows it. */
export interface AgentProcessInfo {
  pid: number | null;
  executable: string | null;
  /** Seconds since the Unix epoch. */
  startedAt: number | null;
  /** Whether this app launched or adopted it, so it can stop it. */
  owned: boolean;
}
export function decodeAgentProcessInfo(value: unknown): AgentProcessInfo {
  const item = record(value, 'agent_process_info response');
  const pid = nullableInteger(item.pid, 'agent_process_info.pid');
  const executable = nullableString(
    item.executable,
    'agent_process_info.executable',
  );
  const startedAt = nullableInteger(
    item.startedAt,
    'agent_process_info.startedAt',
  );
  const owned = bool(item.owned, 'agent_process_info.owned');
  if (pid !== null && pid > 0xffff_ffff)
    throw new Error('agent_process_info.pid must fit a u32');
  if (pid === null && (executable !== null || startedAt !== null || owned))
    throw new Error('agent_process_info metadata requires a process');
  return { pid, executable, startedAt, owned };
}

export interface AppInfo {
  version: string;
  agentSocket: string;
  /** Profile prepared and authenticated by the launcher before opening the app. */
  managedProfile?: string;
  /** macOS Computer Name configured in System Settings. */
  computerName?: string;
}

export interface AppLockState {
  locked: boolean;
  available: boolean;
  mechanism: 'biometry' | 'password' | 'none';
  unavailableReason?: string;
}

export type Unlisten = () => void;

export interface CommandAck {
  ok: true;
}
export function decodeCommandAck(value: unknown): CommandAck {
  if (
    !value ||
    typeof value !== 'object' ||
    Object.keys(value).length !== 1 ||
    !('ok' in value) ||
    value.ok !== true
  )
    throw new Error('Invalid command acknowledgment');
  return { ok: true };
}

export function decodeConnectionLoss(value: unknown): string | null {
  return nullableString(value, 'take_agent_connection_loss response');
}

function rejectVariantFields(
  item: Record<string, unknown>,
  fields: string[],
  at: string,
): void {
  for (const field of fields) {
    if (field in item)
      throw new Error(`${at}.${field} is invalid for this variant`);
  }
}

export function decodeMaintenanceSnapshot(value: unknown): MaintenanceSnapshot {
  const at = 'maintenance snapshot';
  const item = record(value, at);
  const state = string(item.state, `${at}.state`);
  const generation = integer(item.generation, `${at}.generation`);
  const revision = integer(item.revision, `${at}.revision`);
  if (generation < 0) throw new Error(`${at}.generation must be nonnegative`);
  if (revision < 0) throw new Error(`${at}.revision must be nonnegative`);
  if (state === 'idle') {
    rejectVariantFields(
      item,
      ['kind', 'phase', 'operation', 'disposition'],
      at,
    );
    return { state, generation, revision };
  }
  const kind = string(item.kind, `${at}.kind`);
  if (!['export', 'import', 'verify', 'relocate', 'restart'].includes(kind))
    throw new Error(`${at}.kind is invalid`);
  const typedKind = kind as MaintenanceKind;
  if (state === 'active') {
    rejectVariantFields(item, ['operation', 'disposition'], at);
    const phase = string(item.phase, `${at}.phase`);
    if (
      ![
        'selecting',
        'confirming',
        'quiescing',
        'running',
        'restoring',
      ].includes(phase)
    )
      throw new Error(`${at}.phase is invalid`);
    return {
      state,
      generation,
      revision,
      kind: typedKind,
      phase: phase as MaintenancePhase,
    };
  }
  if (state !== 'complete') throw new Error(`${at}.state is invalid`);
  rejectVariantFields(item, ['phase'], at);
  const operationItem = record(item.operation, `${at}.operation`);
  const operationStatus = string(
    operationItem.status,
    `${at}.operation.status`,
  );
  let operation: MaintenanceOperationOutcome;
  if (operationStatus === 'cancelled' || operationStatus === 'completed') {
    rejectVariantFields(operationItem, ['error'], `${at}.operation`);
    operation = { status: operationStatus };
  } else if (operationStatus === 'failed') {
    operation = {
      status: 'failed',
      error: decodeCommandError(operationItem.error, `${at}.operation.error`),
    };
  } else {
    throw new Error(`${at}.operation.status is invalid`);
  }
  const dispositionItem = record(item.disposition, `${at}.disposition`);
  const dispositionStatus = string(
    dispositionItem.status,
    `${at}.disposition.status`,
  );
  let disposition: MaintenanceDisposition;
  if (dispositionStatus === 'continue-current-root') {
    rejectVariantFields(
      dispositionItem,
      ['root', 'error'],
      `${at}.disposition`,
    );
    disposition = { status: dispositionStatus };
  } else if (
    dispositionStatus === 'restart-selected-root' ||
    dispositionStatus === 'recovery-required'
  ) {
    rejectVariantFields(dispositionItem, ['error'], `${at}.disposition`);
    disposition = {
      status: dispositionStatus,
      root: string(dispositionItem.root, `${at}.disposition.root`),
    };
  } else if (dispositionStatus === 'restoration-failed') {
    disposition = {
      status: dispositionStatus,
      root: string(dispositionItem.root, `${at}.disposition.root`),
      error: decodeCommandError(
        dispositionItem.error,
        `${at}.disposition.error`,
      ),
    };
  } else {
    throw new Error(`${at}.disposition.status is invalid`);
  }
  return {
    state,
    generation,
    revision,
    kind: typedKind,
    operation,
    disposition,
  };
}

export function decodeAppInfo(value: unknown): AppInfo {
  const item = record(value, 'app_info response');
  const managedProfile = optionalString(
    item.managedProfile,
    'app_info.managedProfile',
  );
  const computerName = optionalString(
    item.computerName,
    'app_info.computerName',
  );
  return {
    version: string(item.version, 'app_info.version'),
    agentSocket: string(item.agentSocket, 'app_info.agentSocket'),
    ...(managedProfile ? { managedProfile } : {}),
    ...(computerName ? { computerName } : {}),
  };
}

export function decodeAppLockState(value: unknown): AppLockState {
  const item = record(value, 'app_lock_state response');
  const locked = bool(item.locked, 'app_lock_state.locked');
  const available = bool(item.available, 'app_lock_state.available');
  const mechanism = string(item.mechanism, 'app_lock_state.mechanism');
  const unavailableReason = optionalString(
    item.unavailableReason,
    'app_lock_state.unavailableReason',
  );
  if (!['biometry', 'password', 'none'].includes(mechanism)) {
    throw new Error('app_lock_state.mechanism is not supported');
  }
  if (locked && !available) {
    throw new Error('app_lock_state cannot be locked when lock is unavailable');
  }
  if (
    (available && mechanism === 'none') ||
    (!available && mechanism !== 'none')
  ) {
    throw new Error('app_lock_state capability and mechanism are incompatible');
  }
  if ((!available && !unavailableReason) || (available && unavailableReason)) {
    throw new Error('app_lock_state availability reason is inconsistent');
  }
  return {
    locked,
    available,
    mechanism: mechanism as AppLockState['mechanism'],
    ...(unavailableReason === undefined ? {} : { unavailableReason }),
  };
}

export function decodeDropHover(value: unknown): DropHoverEvent {
  const item = record(value, 'foks://drop-hover payload');
  return { hovering: bool(item.hovering, 'foks://drop-hover.hovering') };
}

export function decodeDropPaths(value: unknown): string[] {
  return array(value, 'foks://drop-paths payload', string);
}

export function decodeWindowState(value: unknown): WindowStateEvent {
  const item = record(value, 'foks://window-state payload');
  return {
    maximized: bool(item.maximized, 'foks://window-state.maximized'),
    fullscreen: bool(item.fullscreen, 'foks://window-state.fullscreen'),
  };
}

export function decodeAgentStatus(value: unknown): AgentStatus {
  const item = record(value, 'agent_status response');
  const state = string(item.state, 'agent_status.state');
  if (state === 'ready') {
    rejectVariantFields(item, ['step'], 'agent_status');
    return { state: 'ready' };
  }
  if (state === 'bootstrap') {
    return {
      state: 'bootstrap',
      step: string(item.step, 'agent_status.step'),
    };
  }
  throw new Error('agent_status.state is not ready or bootstrap');
}
