import { useEffect, useRef, useState } from 'react';
import type { Bridge, CheckedServer, ServerStatusSnapshot } from '../../bridge';
import type { AgentSnapshot, Server } from '../../model';
import type { MutationFailureHandler } from '../../mutation-recovery';
import { serverBinding, ServerCheckController } from './server-workflow';

interface Reports {
  bridge: Bridge;
  checked: Map<string, CheckedServer>;
  statuses: Map<string, { server: Server; status: ServerStatusSnapshot }>;
}

/** The reports of this page's own checks, keyed by profile for the caller. */
function byProfile<T>(
  snapshot: AgentSnapshot,
  rows: Map<string, T> | undefined,
): Map<string, T> {
  return new Map(
    snapshot.servers.flatMap((server) => {
      const row = rows?.get(serverBinding(server));
      return row ? [[server.id, row] as const] : [];
    }),
  );
}

export function useServerMetadata({
  bridge,
  snapshot,
  profile,
  enteredScene,
  onMutationError,
  onRefresh,
  toast,
}: {
  bridge: Bridge;
  snapshot: AgentSnapshot;
  profile?: string;
  enteredScene: string;
  onMutationError: MutationFailureHandler;
  onRefresh: (message: string) => Promise<void>;
  toast: (message: string) => void;
}) {
  // A server's signed status is already projected onto its catalog row, so
  // the page states it from the snapshot it is rendering and reads nothing
  // per server. What is held here is the active check's own report: a
  // mutation result, and newer than the catalog refresh it asks for until
  // that refresh lands.
  const [reports, setReports] = useState<Reports>({
    bridge,
    checked: new Map(),
    statuses: new Map(),
  });
  const [busyOwner, setBusyOwner] = useState<ServerCheckController | null>(
    null,
  );
  const latest = useRef({
    bridge,
    snapshot,
    profile,
    onMutationError,
    onRefresh,
    toast,
  });
  latest.current = {
    bridge,
    snapshot,
    profile,
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
    // A report belongs to the bridge it was read through; a new one starts
    // from no reports rather than restating the previous session's.
    const retain = (current: Reports): Reports =>
      current.bridge === bridge
        ? current
        : { bridge, checked: new Map(), statuses: new Map() };
    const controller: ServerCheckController = new ServerCheckController(
      bridge,
      () => latest.current,
      {
        busy: (value) => setBusyOwner(value ? controller : null),
        checked: (binding, report) =>
          setReports((current) => {
            const base = retain(current);
            return {
              ...base,
              checked: new Map(base.checked).set(binding, report),
            };
          }),
        // The check read the signed status back, which is the one fact the
        // catalog will not carry until its refresh lands.
        status: (binding, status) => {
          const server = latest.current.snapshot.servers.find(
            (row) => serverBinding(row) === binding,
          );
          if (!server) return;
          setReports((current) => {
            const base = retain(current);
            return {
              ...base,
              statuses: new Map(base.statuses).set(binding, { server, status }),
            };
          });
        },
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

  const current = reports.bridge === bridge ? reports : undefined;
  return {
    // A check fills the gap until the catalog replaces this server row.
    // Keeping it beyond that point would hide later signed lease updates.
    statuses: new Map(
      snapshot.servers.flatMap((server) => {
        const report = current?.statuses.get(serverBinding(server));
        return report?.server === server
          ? [[server.id, report.status] as const]
          : [];
      }),
    ),
    checked: byProfile(snapshot, current?.checked),
    busy: busyOwner === controller,
    check: (server: Server) => controller.check(server),
  };
}
