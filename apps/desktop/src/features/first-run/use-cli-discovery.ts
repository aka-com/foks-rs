import { useEffect, useMemo, useRef, useState } from 'react';
import type React from 'react';
import { isAgentReadinessError, normalizeCommandError } from '../../bridge';
import type {
  Bridge,
  CommandError,
  GoProfileCandidate,
  GoProfileDiscovery,
} from '../../bridge';
import { sharedSetupRead } from './first-run-loading';
import type { FirstRunStateName } from '../../first-run-state';
import { accountAliasFor } from './use-setup-identity';

/**
 * Owns the scan for official FOKS CLI profiles on this device and the
 * candidate setup continues with: the chooser on "Set up FOKS", and the CLI
 * import and pairing methods on the account steps.
 */
export function useCliDiscovery({
  bridge,
  agentReady,
  state,
  profileHostId,
  accountAlias,
  onAgentReadinessFailure,
  setUsername,
}: {
  bridge: Bridge;
  agentReady: boolean;
  state: FirstRunStateName;
  /** The verified server's host, once a server has been checked. */
  profileHostId: string | undefined;
  accountAlias: string;
  onAgentReadinessFailure?: (error: CommandError) => void;
  setUsername: React.Dispatch<React.SetStateAction<string>>;
}) {
  const [discovery, setDiscovery] = useState<GoProfileDiscovery | null>(null);
  const [candidate, setCandidate] = useState<GoProfileCandidate | null>(null);
  /** Set when the user picks a candidate or continues with the preselected one. */
  const candidateExplicit = useRef(false);
  const [chooserDismissed, setChooserDismissed] = useState(false);
  // The fork on "Set up FOKS": use a CLI account or create a new one. The
  // page exists only when candidates were found, so the existing account is
  // the default.
  const [start, setStart] = useState<'existing' | 'new'>('existing');
  const [scanError, setScanError] = useState<string | null>(null);
  const [scanAttempt, setScanAttempt] = useState(0);

  // CLI profiles are discovered on the joining step for the chooser, and again
  // on the account steps when no result is loaded: a resumed checkpoint skips
  // the joining step, and the CLI import and pairing cards depend on a
  // candidate that is not persisted.
  const accountStep = state === 'account' || state === 'existing';
  const discoveryWanted =
    state === 'who' ? !chooserDismissed : accountStep && discovery === null;
  useEffect(() => {
    if (!agentReady || !bridge.native || !discoveryWanted) return;
    let alive = true;
    setScanError(null);
    void sharedSetupRead(bridge, 'cli-discovery', () =>
      bridge.discoverGoProfiles(),
    )
      .then((result) => {
        if (alive) setDiscovery(result);
      })
      .catch((error) => {
        if (!alive) return;
        const typed = normalizeCommandError(error);
        if (isAgentReadinessError(typed) && onAgentReadinessFailure) {
          onAgentReadinessFailure(typed);
          return;
        }
        setScanError(typed.message);
      });
    return () => {
      alive = false;
    };
  }, [
    agentReady,
    bridge,
    state,
    discoveryWanted,
    chooserDismissed,
    scanAttempt,
    onAgentReadinessFailure,
  ]);

  const candidates = useMemo(
    () =>
      discovery?.candidates.filter((row) => row.pairable || row.copyable) ?? [],
    [discovery],
  );
  // On "Set up FOKS", a single usable candidate is selected without a click.
  const chooserOpen =
    state === 'who' && !chooserDismissed && candidates.length > 0;
  useEffect(() => {
    if (!chooserOpen || candidate || candidates.length !== 1) return;
    setCandidate(candidates[0]);
  }, [chooserOpen, candidate, candidates]);
  // On the account steps, select the CLI profile for the verified server when
  // none was chosen: the alias match wins, otherwise a single profile on that
  // host. Several unrelated profiles on the same host stay unselected rather
  // than importing or pairing the wrong account.
  useEffect(() => {
    if (!accountStep || candidate || !profileHostId) return;
    const onHost = candidates.filter((row) => row.hostId === profileHostId);
    const byAlias = onHost.filter(
      (row) =>
        row.username !== undefined &&
        accountAliasFor(row.username) === accountAlias,
    );
    const match =
      byAlias.length === 1
        ? byAlias[0]
        : onHost.length === 1
          ? onHost[0]
          : undefined;
    if (!match) return;
    setCandidate(match);
    // Mirror the chooser: suggest the CLI username as the account alias.
    if (match.username)
      setUsername(
        (current) => current || accountAliasFor(match.username ?? ''),
      );
  }, [
    accountStep,
    candidate,
    candidates,
    profileHostId,
    accountAlias,
    setUsername,
  ]);

  return {
    discovery,
    candidates,
    candidate,
    candidateExplicit,
    chooserDismissed,
    chooserOpen,
    start,
    setStart,
    scanError,
    /** The joining step is waiting for the scan's first answer. */
    scanPending:
      state === 'who' &&
      Boolean(bridge.native) &&
      !chooserDismissed &&
      !discovery &&
      !scanError,
    /** Picks a candidate in the chooser. */
    choose: (picked: GoProfileCandidate): void => {
      candidateExplicit.current = true;
      setCandidate(picked);
      setStart('existing');
    },
    /**
     * Continues with the selected candidate. Continuing confirms a
     * preselected candidate as well, so the server step keeps it when the
     * address is edited.
     */
    confirm: (): GoProfileCandidate | null => {
      if (candidate) candidateExplicit.current = true;
      return candidate;
    },
    /** Leaves the chooser to create a new account. */
    startNew: (): void => {
      candidateExplicit.current = false;
      setCandidate(null);
      setChooserDismissed(true);
    },
    dismissChooser: (): void => setChooserDismissed(true),
    reopenChooser: (): void => setChooserDismissed(false),
    retryScan: (): void => {
      setDiscovery(null);
      setChooserDismissed(false);
      setScanAttempt((value) => value + 1);
    },
    /** An edited server address no longer implies a candidate nobody picked. */
    addressEdited: (): void => {
      if (!candidateExplicit.current) setCandidate(null);
    },
  };
}

export type CliDiscovery = ReturnType<typeof useCliDiscovery>;
