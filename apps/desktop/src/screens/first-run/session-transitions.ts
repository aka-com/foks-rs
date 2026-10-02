import type { GoProfileCandidate, PendingOperation } from '../../bridge';
import {
  setupBackTarget,
  type FirstRunCheckpoint,
  type FirstRunEvent,
  type FirstRunStateName,
} from '../../first-run-state';

/** Presentation navigation still goes through the persisted checkpoint machine. */
export function setupNavigationEvent(
  current: FirstRunCheckpoint,
  next: FirstRunStateName,
): FirstRunEvent {
  if (next === setupBackTarget(current)) return { type: 'back' };
  if (next === 'account' || next === 'existing')
    return {
      type: 'select-account-method',
      method: next === 'existing' ? 'recover' : 'create',
    };
  return { type: 'navigate', state: next };
}

/** Secret retention is independent of DOM rendering and async preparation. */
export function protectionNavigationPolicy(
  current: Pick<FirstRunCheckpoint, 'state' | 'backupCommitted'>,
  next: FirstRunStateName,
) {
  const leavingProtection = next !== 'protect' && next !== 'phrase';
  const leavingPhrase = current.state === 'phrase' && next !== 'phrase';
  return {
    revealPhrase: next === 'phrase',
    clearPassphrase: leavingProtection,
    discardBackup:
      leavingProtection || (leavingPhrase && !current.backupCommitted),
    resetAcknowledgement: leavingProtection || leavingPhrase,
  };
}

/** Authentication methods for an existing account. */
export type SigninMethod = 'recover' | 'import' | 'pair';

/** A pending operation may suggest a method, but never an unavailable credential source. */
export function selectedSigninMethod({
  candidate,
  chosen,
  pending,
  alias,
}: {
  candidate: Pick<GoProfileCandidate, 'pairable' | 'copyable'> | null;
  chosen: SigninMethod | null;
  pending: readonly PendingOperation[];
  alias: string;
}): SigninMethod | null {
  const methods: SigninMethod[] = [
    ...(candidate?.pairable ? (['pair'] as const) : []),
    ...(candidate?.copyable ? (['import'] as const) : []),
    'recover',
  ];
  const pendingFor = (kind: PendingOperation['kind']) =>
    pending.some((row) => row.kind === kind && row.alias === alias);
  return chosen && methods.includes(chosen)
    ? chosen
    : methods.length === 1
      ? 'recover'
      : methods.includes('pair') && pendingFor('pairing-acceptance')
        ? 'pair'
        : pendingFor('account-recovery')
          ? 'recover'
          : null;
}
