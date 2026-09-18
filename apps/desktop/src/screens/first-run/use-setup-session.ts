import { useState } from 'react';
import type { Bridge } from '../../bridge';
import { normalizeCommandError } from '../../bridge';
import { resolveSetupEntry, setupActions } from '../../first-run-controller';
import {
  persistFirstRun,
  provisioningInFlight,
} from '../../first-run-operations';
import { clearRetainedSetups, retainSetup } from '../../first-run-recovery';
import {
  FIRST_RUN_CHECKPOINT_KEY,
  decodeFirstRunCheckpoint,
  initialFirstRun,
  isFirstRunState,
  type FirstRunCheckpoint,
  type FirstRunPath,
  type FirstRunStateName,
} from '../../first-run-state';
import type { Location } from '../../location';
import type { AgentSnapshot } from '../../model';
import { useFirstRunController } from '../../use-first-run-controller';
import { stepOf } from '../first-run-view';

function fixtureSeed(
  bridge: Bridge,
  path: FirstRunPath,
  state: FirstRunStateName,
): FirstRunCheckpoint {
  const facts = bridge.firstRunFixture?.[path];
  let next = initialFirstRun(path, state);
  if (state === 'boot') return next;
  if (state === 'error' && facts) next = { ...next, serverAddress: facts.typo };
  if (
    (stepOf(state) >= 2 || state === 'checked' || state === 'compare') &&
    facts
  ) {
    next = { ...next, profile: facts.report, serverAddress: facts.server };
  }
  if (stepOf(state) >= 3 && facts) {
    next = {
      ...next,
      account: {
        alias: facts.accountAlias,
        username: facts.username,
        deviceName: facts.deviceName,
      },
    };
  }
  if (state === 'identity-pending' && facts)
    next = {
      ...next,
      provisionedAccount: {
        alias: facts.accountAlias,
        deviceName: facts.deviceName,
      },
    };
  if (stepOf(state) >= 4 && state !== 'phrase')
    next = { ...next, passphraseSet: true, backupCommitted: true };
  if (state === 'added')
    next = {
      ...next,
      added: true,
      group:
        facts?.groupName && facts.groupAlias && facts.groupTeamIdHex
          ? {
              name: facts.groupName,
              kind: 'named',
              alias: facts.groupAlias,
              teamIdHex: facts.groupTeamIdHex,
            }
          : undefined,
    };
  if (state === 'checklist-invited')
    next = {
      ...next,
      passphraseSet: false,
      backupCommitted: false,
      protectSkipped: true,
      added: false,
    };
  if (state === 'checklist-own')
    next = {
      ...next,
      passphraseSet: false,
      backupCommitted: false,
      protectSkipped: true,
      group: undefined,
    };
  return next;
}

export function initialCheckpoint(
  bridge: Bridge,
  snapshot: AgentSnapshot,
  location: Extract<Location, { kind: 'first-run' }>,
  automaticEntry: boolean,
): FirstRunCheckpoint {
  const path: FirstRunPath = location.path ?? 'invited';
  let saved: FirstRunCheckpoint | null = null;
  try {
    saved = decodeFirstRunCheckpoint(
      window.localStorage.getItem(FIRST_RUN_CHECKPOINT_KEY),
    );
  } catch {
    /* unavailable storage */
  }
  const queryState =
    typeof window === 'undefined'
      ? null
      : new URLSearchParams(window.location.search).get('state');
  const namedReviewState = isFirstRunState(queryState);
  if (bridge.firstRunFixture && namedReviewState)
    return fixtureSeed(bridge, path, location.step as FirstRunStateName);
  const state =
    location.step === 'boot' || !isFirstRunState(location.step)
      ? 'who'
      : location.step;
  return resolveSetupEntry(
    snapshot,
    saved,
    { state, path: location.path },
    automaticEntry,
  );
}

export function useSetupCheckpoint({
  sessionEntry,
  automaticEntry,
  ...options
}: Omit<Parameters<typeof useFirstRunController>[0], 'initial'> & {
  sessionEntry: FirstRunCheckpoint | null;
  automaticEntry: boolean;
}) {
  return useFirstRunController({
    ...options,
    initial: () =>
      sessionEntry ??
      initialCheckpoint(
        options.bridge,
        options.snapshot,
        options.location,
        automaticEntry,
      ),
  });
}

export function useSetupSession({
  bridge,
  checkpoint,
  checkpointRef,
  mounted,
  mutationBusy,
  operationRunning,
  clearSecrets,
  invalidateIdentity,
  onReplaceSession,
}: Pick<
  ReturnType<typeof useFirstRunController>,
  'checkpoint' | 'checkpointRef' | 'mounted'
> & {
  bridge: Bridge;
  mutationBusy: boolean;
  operationRunning: boolean;
  clearSecrets: () => void;
  invalidateIdentity: () => void;
  onReplaceSession: (next: FirstRunCheckpoint) => void;
}) {
  const recoveryActions = setupActions(checkpoint, {
    mutating: mutationBusy,
    operationRunning,
    inFlight: Boolean(
      checkpoint.provisioning &&
      provisioningInFlight(bridge, checkpoint.provisioning.id),
    ),
  });
  const [restartError, setRestartError] = useState<string | null>(null);
  const replaceSession = (next: FirstRunCheckpoint): void => {
    persistFirstRun(next);
    clearSecrets();
    mounted.current = false;
    invalidateIdentity();
    onReplaceSession(next);
  };
  /**
   * Setup is re-entered from the app sidebar with an account already set up,
   * so the attempt in hand is retained rather than discarded.
   */
  const reenterSetup = (): void => {
    if (!recoveryActions.canRestart) return;
    try {
      retainSetup(checkpointRef.current);
      replaceSession(initialFirstRun(checkpoint.path));
    } catch (error) {
      setRestartError(normalizeCommandError(error).message);
    }
  };
  const [confirmingRestart, setConfirmingRestart] = useState(false);
  /** Discards this device’s setup, retained attempts included. */
  const startSetupOver = (): void => {
    setConfirmingRestart(false);
    if (!recoveryActions.canRestart) return;
    try {
      clearRetainedSetups();
      replaceSession(initialFirstRun(checkpoint.path));
    } catch (error) {
      setRestartError(normalizeCommandError(error).message);
    }
  };

  return {
    recoveryActions,
    restartError,
    setRestartError,
    confirmingRestart,
    setConfirmingRestart,
    reenterSetup,
    startSetupOver,
  };
}
