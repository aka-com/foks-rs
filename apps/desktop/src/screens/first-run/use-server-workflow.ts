import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type {
  Bridge,
  CheckedProfileResponse,
  GoProfileCandidate,
  ServerStatusSnapshot,
} from '../../bridge';
import { enqueueProfileWork, normalizeCommandError } from '../../bridge';
import type {
  FirstRunFailure,
  FirstRunOperation,
} from '../../first-run-failure';
import type { AgentSnapshot } from '../../model';
import type { useFirstRunController } from '../../use-first-run-controller';
import {
  setupWorkflowScope,
  useSetupWorkflow,
  type WorkflowCheckpoint,
} from './workflow-ownership';

export type WorkflowFailureReporter = (
  operation: FirstRunOperation,
  error: unknown,
  report: (message: string) => void,
  isCurrent: () => boolean,
  onFailure?: (failure: FirstRunFailure) => void,
) => void;

/**
 * The profile identifier a server address suggests: the host as letters,
 * digits, and dashes, keeping a port only when it is not the default. So
 * "foks.app:4430" is `foks-app` and "localhost:5000" is `localhost-5000`.
 */
export function profileNameFor(address: string): string {
  const host = address
    .trim()
    .toLowerCase()
    .replace(/^[a-z]+:\/\//, '')
    .replace(/\/.*$/, '');
  // An IPv6 literal keeps its groups but would otherwise collapse to a run of
  // dashes, so it is named for what it is: "[fe80::1]:4430" is `ipv6-fe80-1`.
  const ipv6 = /^\[([^\]]+)\](?::\d+)?$/.exec(host);
  const bare = ipv6 ? `ipv6-${ipv6[1]}` : host.replace(/:4430$/, '');
  return (
    bare
      .replace(/[^a-z0-9_-]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 52) || 'server'
  );
}

export async function runServerCheck({
  bridge,
  address,
  candidate,
  fixtureProfile,
  isCurrent,
}: {
  bridge: Pick<
    Bridge,
    'checkAndAddGoProfile' | 'checkAndAddProfile' | 'setServerLabel'
  >;
  address: string;
  candidate: GoProfileCandidate | null;
  fixtureProfile?: string;
  isCurrent: () => boolean;
}): Promise<CheckedProfileResponse | null> {
  if (!isCurrent()) return null;
  const profileName = fixtureProfile ?? profileNameFor(address);
  const report = candidate
    ? await bridge.checkAndAddGoProfile(
        candidate.candidateId,
        candidate.hostId,
        profileName,
        address.trim(),
      )
    : await bridge.checkAndAddProfile(profileName, address.trim());
  if (!isCurrent()) return null;
  if (candidate && report.hostId !== candidate.hostId)
    throw new Error('The server response does not match the selected profile.');
  // A profile made here is labeled with the server's own name, so the rail
  // reads "foks.app" rather than the identifier derived from the address.
  if (!fixtureProfile && report.canonicalName) {
    try {
      await bridge.setServerLabel(profileName, report.canonicalName);
    } catch {
      // The label is cosmetic; failing to set it does not fail the check.
    }
  }
  return isCurrent() ? report : null;
}

export function useServerWorkflow({
  bridge,
  agentReady,
  checkpoint,
  checkpointRef,
  mounted,
  send,
  snapshot,
  managedProfile,
  onRefreshSnapshot,
  goCandidate,
  onAddressEdited,
  setMessage,
  fail,
}: WorkflowCheckpoint &
  Pick<ReturnType<typeof useFirstRunController>, 'send'> & {
    bridge: Bridge;
    agentReady: boolean;
    snapshot: AgentSnapshot;
    managedProfile?: string;
    onRefreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
    goCandidate: GoProfileCandidate | null;
    onAddressEdited: () => void;
    setMessage: (message: string | null) => void;
    fail: WorkflowFailureReporter;
  }) {
  const facts = bridge.firstRunFixture?.[checkpoint.path];
  const state = checkpoint.state;
  const inFlight = useRef<(() => boolean) | null>(null);
  const workflow = useSetupWorkflow({
    bridge,
    agentReady,
    checkpoint,
    checkpointRef,
    mounted,
  });
  const [address, setAddressValue] = useState(
    () => checkpoint.serverAddress ?? facts?.server ?? '',
  );
  const [addressInvalid, setAddressInvalid] = useState(false);
  const [serverCheckFailure, setServerCheckFailure] =
    useState<FirstRunFailure | null>(null);
  const serverAddressInput = useRef<HTMLInputElement>(null);
  const addressSelection = useRef<{
    start: number | null;
    end: number | null;
  } | null>(null);
  const [managedStatus, setManagedStatus] =
    useState<ServerStatusSnapshot | null>(null);
  const [managedStatusError, setManagedStatusError] = useState<string | null>(
    null,
  );
  const [managedStatusAttempt, setManagedStatusAttempt] = useState(0);
  const environment = useRef({
    snapshot,
    onRefreshSnapshot,
    managedProfile,
    agentReady,
    bridge,
  });
  environment.current = {
    snapshot,
    onRefreshSnapshot,
    managedProfile,
    agentReady,
    bridge,
  };

  useLayoutEffect(() => {
    const selection = addressSelection.current;
    if (!selection) return;
    addressSelection.current = null;
    serverAddressInput.current?.focus();
    serverAddressInput.current?.setSelectionRange(
      selection.start,
      selection.end,
    );
  }, [address, state]);

  const setAddress = (value: string): void => {
    workflow.invalidate();
    setAddressValue(value);
  };
  const editServerAddress = (value: string): void => {
    const input = serverAddressInput.current;
    if (input && document.activeElement === input)
      addressSelection.current = {
        start: input.selectionStart,
        end: input.selectionEnd,
      };
    onAddressEdited();
    setAddress(value);
    setAddressInvalid(false);
    setMessage(null);
    setServerCheckFailure(null);
    send({ type: 'server-edited', address: value });
  };
  const checkServer = async (): Promise<void> => {
    if (!agentReady || !mounted.current || inFlight.current?.()) return;
    const operation = workflow.capture();
    if (!operation.isCurrent()) return;
    if (!address.trim()) {
      setAddressInvalid(true);
      if (state === 'error') send({ type: 'navigate', state: 'address' });
      operation.finish();
      return;
    }
    inFlight.current = operation.isCurrent;
    setAddressInvalid(false);
    setMessage(null);
    setServerCheckFailure(null);
    try {
      const report = await runServerCheck({
        bridge,
        address,
        candidate: goCandidate,
        fixtureProfile: facts?.profile,
        isCurrent: operation.isCurrent,
      });
      if (!report || !operation.isCurrent()) return;
      send({
        type: 'profile-checked',
        address: address.trim(),
        profile: report,
      });
    } catch (error) {
      if (!operation.isCurrent()) return;
      operation.publish(() => send({ type: 'navigate', state: 'error' }));
      fail(
        'server-check',
        error,
        setMessage,
        operation.isCurrent,
        setServerCheckFailure,
      );
    } finally {
      if (inFlight.current === operation.isCurrent) inFlight.current = null;
      operation.finish();
    }
  };

  useEffect(() => {
    if (!agentReady || state !== 'local') return;
    const savedScope = setupWorkflowScope(checkpointRef.current);
    let alive = true;
    const isCurrent = (): boolean =>
      alive &&
      mounted.current &&
      environment.current.agentReady &&
      environment.current.bridge === bridge &&
      environment.current.managedProfile === managedProfile &&
      setupWorkflowScope(checkpointRef.current) === savedScope;
    setManagedStatus(null);
    if (!managedProfile) {
      setManagedStatusError(
        'No local server is running on this device. Connect to an existing server to continue.',
      );
      return;
    }
    setManagedStatusError(null);
    const server = environment.current.snapshot.servers.find(
      (row) => row.id === managedProfile,
    );
    if (server?.trust.status === 'blocked' || server?.restrictions.length) {
      setManagedStatusError(
        server.trust.status === 'blocked'
          ? server.trust.error.message
          : server.restrictions[0].error.message,
      );
      return;
    }
    if (!isCurrent()) return;
    void enqueueProfileWork<ServerStatusSnapshot | null>(
      bridge,
      managedProfile,
      () =>
        isCurrent()
          ? bridge.describeServerStatus(managedProfile, true)
          : Promise.resolve(null),
    ).then(
      async (status) => {
        if (!status || !isCurrent()) return;
        if (
          status.profile !== managedProfile ||
          status.configuredProbe !== 'localhost:4430' ||
          status.leaseRequired ||
          !status.host
        ) {
          setManagedStatusError('The local server is not ready.');
          return;
        }
        // Cached connectivity failure must not suppress a live local probe.
        // Publish readiness only after the catalog agrees with its pinned host.
        try {
          const refreshed = await environment.current.onRefreshSnapshot(true);
          if (!isCurrent()) return;
          const current = refreshed.servers.filter(
            (row) => row.id === managedProfile,
          );
          if (
            current.length !== 1 ||
            current[0].host_id !== status.host.hostId ||
            current[0].trust.status === 'blocked' ||
            current[0].restrictions.length
          ) {
            setManagedStatusError(
              'The local server responded, but its saved identity or configuration needs attention. Review server settings.',
            );
            return;
          }
          setManagedStatus(status);
        } catch (error) {
          if (isCurrent())
            setManagedStatusError(
              `The local server responded, but its account list could not be refreshed: ${normalizeCommandError(error).message}`,
            );
        }
      },
      (error) => {
        if (isCurrent())
          setManagedStatusError(normalizeCommandError(error).message);
      },
    );
    return () => {
      alive = false;
    };
  }, [
    bridge,
    managedProfile,
    managedStatusAttempt,
    state,
    agentReady,
    checkpointRef,
    mounted,
  ]);

  const managedReport: CheckedProfileResponse | null = useMemo(() => {
    const host = managedStatus?.host;
    if (!managedProfile || !host || managedStatus.profile !== managedProfile)
      return null;
    return {
      profile: managedProfile,
      acceptance: 'unchanged',
      lookupName: host.lookupName,
      canonicalName: host.canonicalName,
      hostId: host.hostId,
      chain: host.chain,
      epoch: host.epoch,
    };
  }, [managedProfile, managedStatus]);
  const selectManagedProfile = (returning = false): void => {
    if (
      !agentReady ||
      !mounted.current ||
      !managedReport ||
      !managedStatus ||
      environment.current.bridge !== bridge ||
      environment.current.managedProfile !== managedProfile ||
      !environment.current.agentReady ||
      setupWorkflowScope(checkpointRef.current) !==
        setupWorkflowScope(checkpoint)
    )
      return;
    const current = snapshot.servers.filter(
      (server) => server.id === managedReport.profile,
    );
    if (
      current.length !== 1 ||
      current[0].host_id !== managedReport.hostId ||
      current[0].trust.status === 'blocked' ||
      current[0].restrictions.length
    ) {
      setManagedStatusError(
        'Server settings changed. Check the local server again.',
      );
      return;
    }
    send({
      type: 'managed-profile-selected',
      address: managedStatus.configuredProbe,
      profile: managedReport,
      returning,
    });
  };

  return {
    address,
    setAddress,
    addressInvalid,
    serverAddressInput,
    editServerAddress,
    checkServer,
    serverCheckFailure,
    managedStatus,
    managedStatusError,
    setManagedStatusAttempt,
    managedReport,
    selectManagedProfile,
    busy: workflow.busy,
  };
}
