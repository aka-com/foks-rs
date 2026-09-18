import { updateRetainedSetup } from './first-run-recovery';
import { normalizeMutationError, type Bridge } from './bridge';
import {
  decodeFirstRunCheckpoint,
  encodeFirstRunCheckpoint,
  FIRST_RUN_CHECKPOINT_KEY,
  transitionFirstRun,
  type FirstRunCheckpoint,
} from './first-run-state';

export const FIRST_RUN_PROGRESS_EVENT = 'foks:first-run-progress';
const running = new WeakMap<Bridge, Set<string>>();

export function provisioningInFlight(bridge: Bridge, id: string): boolean {
  return running.get(bridge)?.has(id) ?? false;
}

export function persistFirstRun(checkpoint: FirstRunCheckpoint): void {
  window.localStorage.setItem(
    FIRST_RUN_CHECKPOINT_KEY,
    encodeFirstRunCheckpoint(checkpoint),
  );
  window.dispatchEvent(new Event(FIRST_RUN_PROGRESS_EVENT));
}

/** Runs a persisted account operation independently of the initiating screen's lifetime. */
export async function executeProvisioning(
  bridge: Bridge,
  saved: FirstRunCheckpoint,
  operation: () => Promise<unknown>,
  resume = false,
): Promise<{ checkpoint: FirstRunCheckpoint; error?: unknown }> {
  const intent = saved.provisioning;
  if (!intent) throw new Error('Missing account operation.');
  let active = running.get(bridge);
  if (!active) {
    active = new Set();
    running.set(bridge, active);
  }
  if (active.has(intent.id)) throw new Error('Account setup is still running.');
  // Failure to save the intent must prevent dispatch, including on first use.
  persistFirstRun(saved);
  active.add(intent.id);
  let next = saved;
  let failure: unknown;
  try {
    await operation();
    next = transitionFirstRun(
      { ...saved, provisioning: undefined },
      {
        type: 'account-provisioned',
        alias: intent.alias,
        deviceName: intent.deviceName,
      },
    );
  } catch (error) {
    failure = error;
    const typed = normalizeMutationError(error);
    // Transport loss may occur after the server applies the operation. Absence
    // from the catalog does not prove failure, so retain the intent for recovery.
    if (
      !resume &&
      !typed.ambiguous &&
      !typed.fatal &&
      ![
        'ambiguous',
        'response-binding',
        'agent-lost',
        'deadline-exceeded',
        'unknown',
        'mutation-in-flight',
      ].includes(typed.code)
    )
      next = { ...saved, state: intent.back, provisioning: undefined };
  } finally {
    active.delete(intent.id);
  }
  const current = decodeFirstRunCheckpoint(
    window.localStorage.getItem(FIRST_RUN_CHECKPOINT_KEY),
  );
  if (current?.provisioning?.id === intent.id) persistFirstRun(next);
  updateRetainedSetup(saved, next);
  return { checkpoint: next, ...(failure ? { error: failure } : {}) };
}
