import { useEffect, useMemo, useRef, useState } from 'react';
import { shouldReportPassiveServerStatusError } from '../../bridge';
import type { Bridge, CheckedServer } from '../../bridge';
import { useDeviceCache } from '../../device-cache';
import type { AgentSnapshot, Server } from '../../model';
import type { MutationFailureHandler } from '../../mutation-recovery';
import { useMetadataQueries, useMetadataRepository } from '../../query-hooks';
import { serverStatusKey, serverStatusQuery } from '../../resources/servers';
import {
  canReadServer,
  serverBinding,
  ServerCheckController,
} from './server-workflow';

export function useServerMetadata({
  bridge,
  snapshot,
  profile,
  enteredScene,
  onError,
  onMutationError,
  onRefresh,
  toast,
}: {
  bridge: Bridge;
  snapshot: AgentSnapshot;
  profile?: string;
  enteredScene: string;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
  onRefresh: (message: string) => Promise<void>;
  toast: (message: string) => void;
}) {
  // Each server's passive status is a repository row, shared with any other
  // reader and kept across visits for the repository's freshness window;
  // the active check's report is this page's own, since it is a mutation
  // result.
  const devices = useDeviceCache();
  const repository = useMetadataRepository(bridge, devices?.repository);
  const readable = useMemo(
    () => snapshot.servers.filter(canReadServer),
    [snapshot.servers],
  );
  const statusQueries = useMemo(
    () =>
      readable.map((server) => serverStatusQuery(repository, bridge, server)),
    [readable, repository, bridge],
  );
  const [checked, setChecked] = useState({
    bridge,
    rows: new Map<string, CheckedServer>(),
  });
  const [busyOwner, setBusyOwner] = useState<ServerCheckController | null>(
    null,
  );
  const latest = useRef({
    bridge,
    snapshot,
    profile,
    repository,
    onError,
    onMutationError,
    onRefresh,
    toast,
  });
  latest.current = {
    bridge,
    snapshot,
    profile,
    repository,
    onError,
    onMutationError,
    onRefresh,
    toast,
  };
  const statusStates = useMetadataQueries(statusQueries, {
    // A passive read's expected failures are not the page's to report.
    onError: (error) => {
      if (shouldReportPassiveServerStatusError(error))
        latest.current.onError(error);
    },
  });
  const scope = JSON.stringify([profile, snapshot.servers.map(serverBinding)]);
  const owner = useRef<{
    bridge: Bridge;
    scope: string;
    controller: ServerCheckController;
  } | null>(null);
  if (
    !owner.current ||
    owner.current.bridge !== bridge ||
    owner.current.scope !== scope
  ) {
    owner.current?.controller.retire();
    const controller: ServerCheckController = new ServerCheckController(
      bridge,
      () => latest.current,
      {
        busy: (value) => setBusyOwner(value ? controller : null),
        checked: (binding, report) =>
          setChecked((current) => ({
            bridge,
            rows: new Map(current.bridge === bridge ? current.rows : []).set(
              binding,
              report,
            ),
          })),
        // The check read the status back; the shared row is asked again so
        // every reader of it sees the checked server, this page included.
        status: (binding) =>
          latest.current.repository.invalidate([...serverStatusKey(binding)]),
        toast: (message) => latest.current.toast(message),
        refresh: (message) => latest.current.onRefresh(message),
        error: (error) => latest.current.onMutationError(error),
      },
    );
    owner.current = { bridge, scope, controller };
  }
  const controller = owner.current.controller;
  useEffect(() => {
    controller.activate();
    return () => controller.retire();
  }, [controller]);

  const seededCheck = useRef(false);
  const selected = snapshot.servers.find((server) => server.id === profile);
  useEffect(() => {
    if (
      !bridge.fixtureSnapshot ||
      enteredScene !== 'servers-check' ||
      !selected ||
      seededCheck.current
    )
      return;
    void controller.check(selected, () => {
      seededCheck.current = true;
    });
  }, [bridge, controller, enteredScene, selected]);

  const statuses = useMemo(
    () =>
      new Map(
        readable.flatMap((server, index) => {
          const status = statusStates[index]?.data;
          return status ? [[server.id, status] as const] : [];
        }),
      ),
    [readable, statusStates],
  );
  return {
    statuses,
    checked: new Map(
      snapshot.servers.flatMap((server) => {
        const report =
          checked.bridge === bridge
            ? checked.rows.get(serverBinding(server))
            : undefined;
        return report ? [[server.id, report] as const] : [];
      }),
    ),
    busy: busyOwner === controller,
    check: (server: Server) => controller.check(server),
  };
}
