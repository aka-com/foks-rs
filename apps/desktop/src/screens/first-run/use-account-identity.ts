import { useEffect, useRef, useState } from 'react';
import { isAgentReadinessError, normalizeCommandError } from '../../bridge';
import type { Bridge, CommandError } from '../../bridge';
import {
  resolveProvisionedIdentity,
  provisionedIdentityProblem,
  identityProblemText,
  type IdentityProblem,
} from '../../first-run-identity';
import { sharedSetupRead } from '../../first-run-loading';
import type { FirstRunCheckpoint } from '../../first-run-state';
import type { AgentSnapshot } from '../../model';
import type { useFirstRunController } from '../../use-first-run-controller';

/**
 * Delay before the first automatic retry of a transient identity refresh
 * failure, doubling from there. The failures this retries -- a native
 * mutation still holding the catalog, a snapshot replaced by a concurrent
 * load -- almost always clear on the first retry, so the first wait stays
 * short and only a genuinely stuck mutation reaches the longer ones.
 */
const IDENTITY_RETRY_DELAY_MS = 2_000;
/**
 * Ceiling on that doubling. Without it the last waits dominate the budget
 * below and a failure that clears late is reported far after it cleared.
 */
const IDENTITY_RETRY_MAXIMUM_DELAY_MS = 8_000;
/**
 * How long to keep retrying in total, so a mutation that never finishes still
 * surfaces its error. The flat two-second retry waited sixty seconds across
 * its thirty attempts; this is that budget rounded up to a whole retry, so
 * patience is not reduced. What changed is the cost of spending it: ten
 * whole-catalog reads rather than thirty-one.
 */
const IDENTITY_RETRY_BUDGET_MS = 62_000;

/**
 * The wait before retry `attempt`, or null once the budget is spent.
 *
 * Every read this schedules is a forced whole-catalog read: the scoped
 * per-profile read is unavailable here, because the reconciliation scheduler
 * that serves it is deliberately not running during first run. So the number
 * of retries is the number of whole-catalog reads, and backing off is what
 * bounds it.
 */
export function identityRetryDelay(attempt: number): number | null {
  let elapsed = 0;
  for (let index = 0; ; index += 1) {
    const delay = Math.min(
      IDENTITY_RETRY_DELAY_MS * 2 ** index,
      IDENTITY_RETRY_MAXIMUM_DELAY_MS,
    );
    if (index === attempt)
      return elapsed + delay > IDENTITY_RETRY_BUDGET_MS ? null : delay;
    elapsed += delay;
    if (elapsed >= IDENTITY_RETRY_BUDGET_MS) return null;
  }
}

/**
 * Text to show while an identity refresh failure that clears on its own is
 * retried, or null for failures that need the user.
 */
function transientIdentityFailure(code: string): string | null {
  switch (code) {
    case 'mutation-in-flight':
      return 'Waiting for the current operation to finish…';
    case 'catalog-required':
      return 'The vault changed while loading. Trying again…';
    default:
      return null;
  }
}

export function useAccountIdentity({
  bridge,
  agentReady,
  onRefreshSnapshot,
  onAgentReadinessFailure,
  checkpointRef,
  mounted,
  commit,
}: Pick<
  ReturnType<typeof useFirstRunController>,
  'checkpointRef' | 'mounted' | 'commit'
> & {
  bridge: Bridge;
  agentReady: boolean;
  onRefreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  onAgentReadinessFailure?: (error: CommandError) => void;
}) {
  const [identityLoading, setIdentityLoading] = useState(false);
  const [identityError, setIdentityError] = useState<string | null>(null);
  // Shown while a transient identity refresh failure is retried on its own.
  const [identityWaiting, setIdentityWaiting] = useState<string | null>(null);
  const identityRetry = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (identityRetry.current !== null)
        window.clearTimeout(identityRetry.current);
    },
    [],
  );
  const [identityProblem, setIdentityProblem] =
    useState<IdentityProblem | null>(null);
  const autoProbedKey = useRef<string | null>(null);
  const identityGeneration = useRef(0);

  useEffect(() => {
    if (!agentReady) setIdentityLoading(false);
    const generation = identityGeneration;
    return () => {
      generation.current++;
    };
  }, [agentReady]);

  const refreshAccountIdentity = async (
    saved: FirstRunCheckpoint = checkpointRef.current,
    attempt = 0,
  ): Promise<void> => {
    if (!saved.provisionedAccount || !agentReady) return;
    autoProbedKey.current = `identity:${saved.provisionedAccount.alias}`;
    const generation = ++identityGeneration.current;
    if (identityRetry.current !== null) {
      window.clearTimeout(identityRetry.current);
      identityRetry.current = null;
    }
    let retrying = false;
    setIdentityLoading(true);
    setIdentityError(null);
    setIdentityWaiting(null);
    setIdentityProblem(null);
    try {
      // Account mutations invalidate the native catalog. Do not join an
      // in-flight read that may have started before the account was copied or
      // created, or identity adoption can remain stuck on that stale result.
      const refreshed = await sharedSetupRead(
        bridge,
        `identity:${saved.profile?.profile}:${saved.provisionedAccount.alias}`,
        () => onRefreshSnapshot(true),
      );
      if (
        generation !== identityGeneration.current ||
        checkpointRef.current !== saved
      )
        return;
      const resolved = resolveProvisionedIdentity(refreshed, saved);
      if (resolved !== saved) commit(resolved);
      else {
        const problem =
          provisionedIdentityProblem(refreshed, saved) ??
          'inventory-unavailable';
        setIdentityProblem(problem);
        setIdentityError(identityProblemText[problem]);
      }
    } catch (error) {
      if (
        generation !== identityGeneration.current ||
        checkpointRef.current !== saved
      )
        return;
      const typed = normalizeCommandError(error);
      if (isAgentReadinessError(typed)) {
        setIdentityError(typed.message);
        onAgentReadinessFailure?.(typed);
        return;
      }
      // A native mutation still holding the catalog, or a snapshot replaced
      // by a concurrent load, clears on its own. Wait and try again rather
      // than presenting a vault-screen message as a setup failure.
      const waiting = transientIdentityFailure(typed.code);
      const delay = identityRetryDelay(attempt);
      if (waiting && delay !== null) {
        retrying = true;
        setIdentityWaiting(waiting);
        identityRetry.current = window.setTimeout(() => {
          identityRetry.current = null;
          if (!mounted.current || generation !== identityGeneration.current)
            return;
          if (checkpointRef.current !== saved) {
            setIdentityWaiting(null);
            setIdentityLoading(false);
            return;
          }
          void refreshAccountIdentity(saved, attempt + 1);
        }, delay);
        return;
      }
      setIdentityError(typed.message);
    } finally {
      if (generation === identityGeneration.current && !retrying)
        setIdentityLoading(false);
    }
  };

  return {
    identityLoading,
    setIdentityLoading,
    identityError,
    identityWaiting,
    identityProblem,
    autoProbedKey,
    refreshAccountIdentity,
    invalidateIdentity: () => {
      identityGeneration.current++;
    },
    clearIdentityProblem: () => {
      setIdentityError(null);
      setIdentityProblem(null);
    },
  };
}
