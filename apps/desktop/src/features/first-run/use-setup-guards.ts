import { useCallback, useLayoutEffect, useRef } from 'react';
import type { FirstRunStateName } from '../../first-run-state';
import type { NavigationGuard } from '../../location';
import { useNavigationGuard } from '../../navigation-guard';
import { useConcealOnInactive } from '../../use-conceal-on-inactive';
import type { SetupSecrets } from './use-setup-secrets';

/**
 * Terminal first-run states where account setup is complete, all user input is
 * committed, and navigation to the main application is active. These states do
 * not prompt for confirmation when navigating away.
 */
const FIRST_RUN_SETTLED: readonly FirstRunStateName[] = [
  'added',
  'local-done',
  'checklist-invited',
  'checklist-own',
];

/**
 * The effects that keep setup's secrets from outliving the step that holds
 * them: the navigation guard, clearing them when the agent becomes unavailable
 * or the shell conceals, and concealing the recovery phrase while the window
 * is inactive. Clearing on the phrase step returns to protect, since the
 * phrase it showed is gone.
 */
export function useSetupGuards({
  state,
  agentReady,
  concealSignal,
  secrets,
  leaveBlocked,
  go,
}: {
  state: FirstRunStateName;
  agentReady: boolean;
  /** Changes when the shell conceals secrets on screen. */
  concealSignal: number;
  secrets: SetupSecrets;
  /** Whether Leave setup is refused because a write is out. */
  leaveBlocked: boolean;
  go: (next: FirstRunStateName) => void;
}): void {
  const { clear: clearSecrets, held } = secrets;
  const { setRecoveryPhraseConcealed } = secrets.backup;

  /**
   * Setup is left through the sidebar, which clears what was typed on the way
   * out. The rail and Control-Tab are not on screen until the end steps, but
   * ⌘K and the back swipe are, and neither of them clears anything: a step
   * holding a secret or an unfinished write answers for them here. A move
   * within setup is the step machine publishing its own address, and the end
   * steps are past everything this protects.
   */
  const setupGuard = useCallback<NavigationGuard>(
    (intent) =>
      (intent.kind === 'navigate' && intent.location.kind === 'first-run') ||
      FIRST_RUN_SETTLED.includes(state) ||
      (!held.current && !leaveBlocked)
        ? null
        : { verdict: 'refuse', reason: 'Finish or leave setup first.' },
    [leaveBlocked, held, state],
  );
  useNavigationGuard(setupGuard);

  useLayoutEffect(() => {
    if (agentReady) return;
    clearSecrets();
    if (state === 'phrase') go('protect');
  }, [agentReady, clearSecrets, go, state]);

  const lastConcealSignal = useRef(concealSignal);
  useLayoutEffect(() => {
    if (concealSignal === lastConcealSignal.current) return;
    lastConcealSignal.current = concealSignal;
    clearSecrets();
    if (state === 'phrase') go('protect');
  }, [clearSecrets, concealSignal, go, state]);

  useConcealOnInactive(
    () => setRecoveryPhraseConcealed(true),
    state === 'phrase',
  );
}
