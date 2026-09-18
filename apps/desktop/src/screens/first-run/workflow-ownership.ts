import { useLayoutEffect, useRef, useState } from 'react';
import { AccessLifetime, type AccessTicket } from '../../app/access-lifetime';
import type { Bridge } from '../../bridge';
import type { FirstRunCheckpoint } from '../../first-run-state';
import type { useFirstRunController } from '../../use-first-run-controller';

export type WorkflowCheckpoint = Pick<
  ReturnType<typeof useFirstRunController>,
  'checkpoint' | 'checkpointRef' | 'mounted'
>;

export function setupWorkflowScope(checkpoint: FirstRunCheckpoint): string {
  return JSON.stringify([
    checkpoint.path,
    checkpoint.state,
    checkpoint.serverAddress,
    checkpoint.profile?.profile,
    checkpoint.profile?.hostId,
    checkpoint.account?.alias,
    checkpoint.provisioning?.id,
    checkpoint.provisionedAccount?.alias,
    checkpoint.selectedGroup,
  ]);
}

export function captureSetupWorkflow(
  ticket: AccessTicket,
  checkpoint: FirstRunCheckpoint,
  current: () => FirstRunCheckpoint,
  available: () => boolean,
): () => boolean {
  const scope = setupWorkflowScope(checkpoint);
  return () =>
    ticket.isCurrent() &&
    available() &&
    setupWorkflowScope(current()) === scope;
}

export function releaseSetupWorkflow(
  active: (() => boolean) | null,
  finished: () => boolean,
): (() => boolean) | null {
  return active === finished ? null : active;
}

export function useSetupWorkflow({
  checkpoint,
  checkpointRef,
  mounted,
  bridge,
  agentReady,
}: WorkflowCheckpoint & { bridge: Bridge; agentReady: boolean }) {
  const scope = setupWorkflowScope(checkpoint);
  const generation = useRef({
    scope,
    bridge,
    agentReady,
    lifetime: new AccessLifetime(),
  });
  if (
    generation.current.scope !== scope ||
    generation.current.bridge !== bridge ||
    generation.current.agentReady !== agentReady
  ) {
    generation.current.lifetime.retire('access-change');
    generation.current = {
      scope,
      bridge,
      agentReady,
      lifetime: new AccessLifetime(),
    };
  }
  useLayoutEffect(
    () => () => {
      generation.current.lifetime.retire('access-change');
    },
    [],
  );
  const [active, setActive] = useState<(() => boolean) | null>(null);
  const invalidate = (): void => {
    generation.current.lifetime.retire('access-change');
    setActive(null);
  };
  const capture = () => {
    const owner = generation.current;
    if (
      owner.scope !== scope ||
      owner.bridge !== bridge ||
      !owner.agentReady ||
      !mounted.current ||
      setupWorkflowScope(checkpointRef.current) !== scope
    )
      return {
        isCurrent: () => false,
        publish: () => {},
        finish: () => {},
      };
    owner.lifetime.retire('access-change');
    const ticket = owner.lifetime.capture();
    const available = () =>
      mounted.current && owner.agentReady && generation.current === owner;
    let current = captureSetupWorkflow(
      ticket,
      checkpoint,
      () => checkpointRef.current,
      available,
    );
    const isCurrent = () => current();
    setActive(() => isCurrent);
    return {
      isCurrent,
      publish: (update: () => void) => {
        if (!isCurrent()) return;
        update();
        owner.scope = setupWorkflowScope(checkpointRef.current);
        current = captureSetupWorkflow(
          ticket,
          checkpointRef.current,
          () => checkpointRef.current,
          available,
        );
      },
      finish: () => {
        if (mounted.current)
          setActive((current) => releaseSetupWorkflow(current, isCurrent));
      },
    };
  };
  return { capture, invalidate, busy: Boolean(active?.()) };
}
