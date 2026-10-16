/** Restarts the agent, alone or so that a refused maintenance action runs. */

import { CollectionStatus } from '../../components/collection-status';
import type { CollectionReadiness } from '../../model';
import { useState } from 'react';
import type { ReactNode } from 'react';
import { normalizeCommandError } from '../../bridge';
import type { AgentProcessInfo, Bridge } from '../../bridge';
import { Button, Inset, InsetRow, SheetDialog } from '../../components';

/** Why the agent is being restarted: for its own sake, or to enable a
 *  maintenance action the agent's ownership refused. */
export type RestartPurpose =
  'restart' | 'export' | 'import' | 'verify' | 'relocate';

export interface RestartRequest {
  purpose: RestartPurpose;
  /** The refused action, run again once the agent is this app's. */
  then?: () => Promise<unknown>;
}

const RESTART_COPY: Readonly<
  Record<Exclude<RestartPurpose, 'restart'>, { title: string; enable: string }>
> = {
  export: { title: 'Restart the agent to export?', enable: 'export' },
  import: { title: 'Restart the agent to import?', enable: 'import' },
  verify: {
    title: 'Restart the agent to verify?',
    enable: 'verification',
  },
  relocate: {
    title: 'Restart the agent to move FOKS data?',
    enable: 'the move',
  },
};

export function isExternalAgent(error: unknown): boolean {
  return normalizeCommandError(error).code === 'external-agent';
}

/** "Started today, 16:26" / "Started yesterday, 16:26" / "Started Sep 16, 16:26". */
function startedLabel(startedAt: number, now = Date.now()): string {
  const started = new Date(startedAt * 1000);
  const time = started.toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  });
  const day = (date: Date): number =>
    new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  const days = Math.round((day(new Date(now)) - day(started)) / 86_400_000);
  const when =
    days === 0
      ? 'today'
      : days === 1
        ? 'yesterday'
        : started.toLocaleDateString(undefined, {
            month: 'short',
            day: 'numeric',
          });
  return `Started ${when}, ${time}`;
}

/**
 * Restart the agent: stop the process on the socket and start this app's
 * own. A plain restart asks once; a restart in service of a maintenance
 * action says which, and runs that action once the new agent is up.
 */
export function RestartAgentSheet({
  bridge,
  request,
  process,
  processReadiness,
  onRetryInfo,
  onClose,
  onError,
}: {
  bridge: Bridge;
  request: RestartRequest;
  process: AgentProcessInfo | null;
  processReadiness: CollectionReadiness;
  onRetryInfo: () => void;
  onClose: () => void;
  onError: (error: unknown) => void;
}): ReactNode {
  const [busy, setBusy] = useState(false);
  const copy =
    request.purpose === 'restart' ? null : RESTART_COPY[request.purpose];
  const plain = copy === null;
  const running =
    process?.pid !== null && process?.pid !== undefined
      ? `foks-agent (PID ${process.pid})` +
        (process.startedAt !== null
          ? ` · ${startedLabel(process.startedAt)}`
          : '')
      : 'No agent is answering on the socket.';
  return (
    <SheetDialog
      width="wide"
      dismissible={!busy}
      onClose={() => {
        if (!busy) onClose();
      }}
      title={copy ? copy.title : 'Restart the agent?'}
      icon="refresh"
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            busy={busy}
            disabled={busy}
            onClick={() => {
              setBusy(true);
              void (async () => {
                try {
                  // A plain restart of an agent we own needs no takeover; a
                  // foreign one does, and so does any restart made to enable
                  // an action the ownership check refused.
                  await bridge.restartAgent(
                    !plain || Boolean(process && !process.owned),
                  );
                  // The shell toasts the restart's outcome from the
                  // maintenance event; only the follow-on action is this
                  // sheet's to run.
                  onClose();
                  if (request.then) await request.then();
                } catch (error) {
                  onError(error);
                  onClose();
                }
              })();
            }}
          >
            {busy ? 'Restarting…' : 'Restart agent'}
          </Button>
        </>
      }
    >
      <p>
        {copy
          ? `The running foks-agent was not started by this app. Stop it, and start a new agent, to enable ${copy.enable}?`
          : 'FOKS will stop the local agent and start it again. This will take a few seconds.'}
      </p>
      <Inset>
        <InsetRow label="Running agent">
          {processReadiness === 'ready' ? (
            running
          ) : (
            <>
              <CollectionStatus
                state={processReadiness}
                label="agent information"
              />
              {processReadiness === 'unavailable' ? (
                <Button onClick={onRetryInfo}>Retry</Button>
              ) : null}
            </>
          )}
        </InsetRow>
        {process?.executable ? (
          <InsetRow label="Launched from" valueClass="mono">
            {process.executable}
          </InsetRow>
        ) : null}
        {plain ? (
          <InsetRow label="In progress">Nothing in progress.</InsetRow>
        ) : null}
      </Inset>
    </SheetDialog>
  );
}
