import { useRef, useState } from 'react';
import {
  enqueueProfileWork,
  type Bridge,
  type DiscoveredGroup,
} from '../../bridge';
import {
  transitionFirstRun,
  type FirstRunCheckpoint,
} from '../../first-run-state';
import { storeReadable, type AgentSnapshot } from '../../model';
import type { useFirstRunController } from '../../use-first-run-controller';
import type { WorkflowFailureReporter } from './use-server-workflow';
import { useSetupWorkflow, type WorkflowCheckpoint } from './workflow-ownership';

type DiscoveryResult = Awaited<ReturnType<Bridge['discoverGroups']>>;

export async function runTeamDiscovery({
  bridge,
  saved,
  isCurrent,
  onRefreshSnapshot,
}: {
  bridge: Bridge;
  saved: FirstRunCheckpoint;
  isCurrent: () => boolean;
  onRefreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
}): Promise<{
  result: DiscoveryResult;
  refreshed: AgentSnapshot;
} | null> {
  const profile = saved.profile;
  const account = saved.account;
  if (!profile || !account || !isCurrent()) return null;
  let attempted = false;
  let refreshed: AgentSnapshot | undefined;
  try {
    let result: DiscoveryResult | null;
    try {
      result = await enqueueProfileWork<DiscoveryResult | null>(
        bridge,
        profile.profile,
        () => {
          if (!isCurrent()) return Promise.resolve(null);
          attempted = true;
          return bridge.discoverGroups(profile.profile, account.alias);
        },
      );
    } finally {
      // Discovery invalidates the catalog even for zero groups or errors.
      if (attempted) refreshed = await onRefreshSnapshot(true);
    }
    if (!isCurrent() || !result || !refreshed) return null;
    if (
      result.accountAlias !== account.alias ||
      result.groups.some((row) => row.accountAlias !== account.alias)
    )
      throw new Error('Team discovery returned data for a different account.');
    return { result, refreshed };
  } catch (error) {
    if (!isCurrent()) return null;
    throw error;
  }
}

export function useTeamDiscovery({
  bridge,
  agentReady,
  checkpoint,
  checkpointRef,
  mounted,
  commit,
  busy,
  onRefreshSnapshot,
  setMessage,
  fail,
}: WorkflowCheckpoint &
  Pick<ReturnType<typeof useFirstRunController>, 'commit'> & {
    bridge: Bridge;
    agentReady: boolean;
    busy: boolean;
    onRefreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
    setMessage: (message: string | null) => void;
    fail: WorkflowFailureReporter;
  }) {
  const workflow = useSetupWorkflow({
    bridge,
    agentReady,
    checkpoint,
    checkpointRef,
    mounted,
  });
  const [choices, setChoices] = useState<{
    groups: DiscoveredGroup[];
    snapshot: AgentSnapshot;
    isCurrent: () => boolean;
  } | null>(null);
  const inFlight = useRef<(() => boolean) | null>(null);

  const selectGroup = (
    found: DiscoveredGroup,
    refreshed: AgentSnapshot,
  ): void => {
    const saved = checkpointRef.current;
    const profile = saved.profile;
    if (!mounted.current || !agentReady || !profile || !saved.account) return;
    if (
      !bridge.firstRunFixture &&
      refreshed.servers.filter(
        (server) =>
          server.id === profile.profile && server.host_id === profile.hostId,
      ).length !== 1
    ) {
      setMessage(
        'The server identity could not be confirmed. Review server settings before opening this team.',
      );
      return;
    }
    const identity = {
      name: found.name ?? found.alias,
      kind: found.kind,
      alias: found.alias,
      teamIdHex: found.teamIdHex,
    };
    const selected = {
      ...saved,
      state: 'waiting' as const,
      selectedGroup: identity,
    };
    const stores = refreshed.stores.filter(
      (store) =>
        store.kind === 'team' &&
        store.active &&
        store.server === profile.profile &&
        store.account === saved.account?.alias &&
        store.team_id_hex === found.teamIdHex &&
        store.alias === found.alias &&
        store.team_kind === found.kind,
    );
    if (stores.length !== 1 || !storeReadable(refreshed, stores[0].id)) {
      commit(selected);
      setMessage(
        'Team located, but the vault is currently unavailable. Retry loading the vault, or complete setup later.',
      );
      return;
    }
    commit(
      transitionFirstRun(selected, {
        type: 'group-discovered',
        group: { ...identity, name: stores[0].name },
      }),
    );
  };
  const selectDiscoveredGroup = (found: DiscoveredGroup): void => {
    if (!choices?.isCurrent() || !choices.groups.includes(found)) return;
    selectGroup(found, choices.snapshot);
  };
  const discover = async (): Promise<void> => {
    const saved = checkpointRef.current;
    if (
      !agentReady ||
      !mounted.current ||
      !saved.profile ||
      !saved.account ||
      busy ||
      inFlight.current?.()
    )
      return;
    const operation = workflow.capture();
    if (!operation.isCurrent()) return;
    inFlight.current = operation.isCurrent;
    setMessage(null);
    setChoices(null);
    try {
      const outcome = await runTeamDiscovery({
        bridge,
        saved,
        isCurrent: operation.isCurrent,
        onRefreshSnapshot,
      });
      if (!outcome || !operation.isCurrent()) return;
      const { result, refreshed } = outcome;
      const facts = bridge.firstRunFixture?.[saved.path];
      const eligible = result.groups.filter(
        (candidate) =>
          candidate.active &&
          (!facts?.groupName || candidate.name === facts.groupName),
      );
      const unique = new Set(
        eligible.map((row) => `${row.kind}:${row.teamIdHex}:${row.alias}`),
      );
      if (unique.size !== eligible.length)
        throw new Error('Team discovery returned conflicting team records.');
      const selected = saved.selectedGroup;
      const match = selected
        ? eligible.filter(
            (row) =>
              row.teamIdHex === selected.teamIdHex &&
              row.alias === selected.alias &&
              row.kind === selected.kind,
          )
        : eligible;
      if (match.length === 1) selectGroup(match[0], refreshed);
      else if (match.length > 1) {
        setChoices({
          groups: match,
          snapshot: refreshed,
          isCurrent: operation.isCurrent,
        });
        setMessage(
          'Choose the team you want to open. Your other memberships will remain available.',
        );
      } else
        setMessage(
          selected
            ? 'Membership in the selected team could not be confirmed. Check again or choose another team.'
            : 'No active teams found yet. You can use Personal while you wait.',
        );
    } catch (error) {
      if (operation.isCurrent())
        fail('group-discovery', error, setMessage, operation.isCurrent);
    } finally {
      if (inFlight.current === operation.isCurrent) inFlight.current = null;
      operation.finish();
    }
  };
  return {
    discover,
    selectDiscoveredGroup,
    discoveredGroups: choices?.isCurrent() ? choices.groups : [],
    busy: workflow.busy,
  };
}
