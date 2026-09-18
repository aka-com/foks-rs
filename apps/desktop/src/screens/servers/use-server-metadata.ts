import { useEffect, useRef, useState } from 'react';
import { AccessLifetime } from '../../app/access-lifetime';
import type { Bridge, CheckedServer, ServerStatusSnapshot } from '../../bridge';
import type { AgentSnapshot, Server } from '../../model';
import type { MutationFailureHandler } from '../../mutation-recovery';
import {
  loadServerStatuses,
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
  const [statuses, setStatuses] = useState({
    bridge,
    rows: new Map<string, ServerStatusSnapshot>(),
  });
  const [checked, setChecked] = useState({
    bridge,
    rows: new Map<string, CheckedServer>(),
  });
  const [busyOwner, setBusyOwner] = useState<ServerCheckController | null>(null);
  const latest = useRef({
    bridge,
    snapshot,
    profile,
    onError,
    onMutationError,
    onRefresh,
    toast,
  });
  latest.current = {
    bridge,
    snapshot,
    profile,
    onError,
    onMutationError,
    onRefresh,
    toast,
  };
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
        status: (binding, status) =>
          setStatuses((current) => ({
            bridge,
            rows: new Map(current.bridge === bridge ? current.rows : []).set(
              binding,
              status,
            ),
          })),
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

  useEffect(() => {
    const lifetime = new AccessLifetime();
    const ticket = lifetime.capture();
    const servers = snapshot.servers;
    const isCurrent = () =>
      ticket.isCurrent() &&
      bridge === latest.current.bridge &&
      servers === latest.current.snapshot.servers;
    void loadServerStatuses(
      bridge,
      servers,
      isCurrent,
      (rows) => setStatuses({ bridge, rows }),
      (error) => latest.current.onError(error),
    );
    return () => lifetime.dispose();
  }, [bridge, snapshot.servers]);

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

  return {
    statuses: new Map(
      snapshot.servers.flatMap((server) => {
        const status =
          statuses.bridge === bridge
            ? statuses.rows.get(serverBinding(server))
            : undefined;
        return status ? [[server.id, status] as const] : [];
      }),
    ),
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
