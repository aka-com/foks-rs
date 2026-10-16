import { useState } from 'react';
import { normalizeCommandError } from '../../bridge';
import { setupActions } from './first-run-controller';
import { persistFirstRun, provisioningInFlight } from './first-run-operations';
import { clearRetainedSetups, retainSetup } from './first-run-recovery';
import {
  initialFirstRun,
  type FirstRunCheckpoint,
} from '../../first-run-state';
import type { SetupCore } from './setup-core';

/**
 * Restarting and re-entering setup: both replace this session with a fresh
 * checkpoint, and differ in whether the attempt in hand is retained.
 */
export function useSetupRestart({
  core,
  mutationBusy,
  operationRunning,
  clearSecrets,
  invalidateIdentity,
  onReplaceSession,
}: {
  core: SetupCore;
  mutationBusy: boolean;
  operationRunning: boolean;
  clearSecrets: () => void;
  invalidateIdentity: () => void;
  onReplaceSession: (next: FirstRunCheckpoint) => void;
}) {
  const { bridge, checkpoint, checkpointRef, mounted } = core;
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

export type SetupRestart = ReturnType<typeof useSetupRestart>;
