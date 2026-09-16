import { useCallback, useEffect, useRef, useState } from 'react';
import type { Bridge } from './bridge';
import type { AgentSnapshot } from './model';
import type { Location } from './location';
import { reconcileSetup, resolveSetupEntry } from './first-run-controller';
import { updateRetainedSetup } from './first-run-recovery';
import { FIRST_RUN_PROGRESS_EVENT } from './first-run-operations';
import {
  FIRST_RUN_CHECKPOINT_KEY,
  normalizeFirstRunCheckpoint,
  decodeFirstRunCheckpoint,
  encodeFirstRunCheckpoint,
  isFirstRunState,
  transitionFirstRun,
  type FirstRunCheckpoint,
} from './first-run-state';

/** Owns the wizard checkpoint, its navigation projection, and authoritative updates. */
export function useFirstRunController({
  initial,
  snapshot,
  bridge,
  location,
  onNavigate,
  agentReady,
}: {
  initial: () => FirstRunCheckpoint;
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'first-run' }>;
  onNavigate: (location: Location) => void;
  agentReady: boolean;
}) {
  const [checkpoint, setCheckpoint] = useState(() =>
    normalizeFirstRunCheckpoint(initial()),
  );
  const checkpointRef = useRef(checkpoint);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const commit = useCallback(
    (candidate: FirstRunCheckpoint): void => {
      if (!mounted.current) return;
      const previous = checkpointRef.current;
      const next = normalizeFirstRunCheckpoint(candidate);
      checkpointRef.current = next;
      setCheckpoint(next);
      try {
        if (next.account) updateRetainedSetup(previous, next);
        window.localStorage.setItem(
          FIRST_RUN_CHECKPOINT_KEY,
          encodeFirstRunCheckpoint(next),
        );
      } catch {
        /* unavailable storage */
      }
      onNavigate({ kind: 'first-run', step: next.state, path: next.path });
    },
    [onNavigate],
  );
  const requestedRoute = `${location.path ?? ''}:${location.step}`;
  const lastRequestedRoute = useRef<string | null>(null);
  useEffect(() => {
    if (lastRequestedRoute.current === null) {
      lastRequestedRoute.current = requestedRoute;
      const current = checkpointRef.current;
      if (
        location.step !== current.state ||
        (location.path && location.path !== current.path)
      )
        commit(current);
      return;
    }
    if (lastRequestedRoute.current === requestedRoute) return;
    lastRequestedRoute.current = requestedRoute;
    const current = checkpointRef.current;
    if (
      location.step === current.state &&
      (!location.path || location.path === current.path)
    )
      return;
    const next = resolveSetupEntry(snapshot, current, {
      state: isFirstRunState(location.step) ? location.step : 'who',
      path: location.path,
    });
    commit(next);
  }, [requestedRoute, location.step, location.path, snapshot, commit]);
  const lastReconciledSnapshot = useRef(snapshot);
  useEffect(() => {
    const update = () => {
      const current = checkpointRef.current;
      if (!current.provisioning) return;
      const next = decodeFirstRunCheckpoint(
        window.localStorage.getItem(FIRST_RUN_CHECKPOINT_KEY),
      );
      if (
        next &&
        next.profile?.hostId === current.profile?.hostId &&
        next.profile?.profile === current.profile?.profile &&
        (next.provisioning?.id === current.provisioning.id ||
          next.provisionedAccount?.alias === current.provisioning.alias ||
          next.state === current.provisioning.back)
      )
        commit(next);
    };
    window.addEventListener(FIRST_RUN_PROGRESS_EVENT, update);
    return () => window.removeEventListener(FIRST_RUN_PROGRESS_EVENT, update);
  }, [commit]);
  useEffect(() => {
    if (!agentReady || bridge.firstRunFixture) return;
    if (lastReconciledSnapshot.current === snapshot) return;
    lastReconciledSnapshot.current = snapshot;
    const reconciled = reconcileSetup(snapshot, checkpoint);
    if (reconciled !== checkpoint) commit(reconciled);
  }, [agentReady, bridge.firstRunFixture, checkpoint, commit, snapshot]);
  const send = useCallback(
    (event: Parameters<typeof transitionFirstRun>[1]): void => {
      commit(transitionFirstRun(checkpointRef.current, event));
    },
    [commit],
  );
  return { checkpoint, checkpointRef, mounted, commit, send };
}
