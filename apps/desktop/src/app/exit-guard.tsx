import { useEffect, useState, type ReactNode } from 'react';
import type { Bridge, ExitAction, ExitState } from '../bridge';
import { normalizeCommandError } from '../bridge';
import { Button, Icon } from '../components';

const IDLE: ExitState = { state: 'idle' };

function unsentCopy(count: number): string | null {
  if (count === 0) return null;
  return count === 1
    ? '1 message has not been sent and will be discarded if you quit.'
    : `${count} messages have not been sent and will be discarded if you quit.`;
}

function ExitOverlay({
  state,
  bridge,
}: {
  state: Exclude<ExitState, { state: 'idle' }>;
  bridge: Bridge;
}): ReactNode {
  const [sending, setSending] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const act = (action: ExitAction): void => {
    if (sending) return;
    setSending(true);
    setFailure(null);
    void bridge
      .handleExitAction(action)
      .catch((error) => setFailure(normalizeCommandError(error).message))
      .finally(() => setSending(false));
  };

  if (state.state === 'stopping' || state.state === 'finalizing') {
    const stopping = state.state === 'stopping';
    return (
      <div className="exit-takeover solid" role="alertdialog" aria-modal="true">
        <div className="exit-progress" role="status">
          <span className="spin" aria-hidden="true" />
          <b>
            {stopping
              ? state.force
                ? 'Force stopping FOKS Agent…'
                : 'Stopping FOKS Agent…'
              : 'Finishing up…'}
          </b>
          <span>
            {stopping
              ? 'FOKS will quit after the background agent has stopped.'
              : 'Clearing sensitive clipboard data and closing FOKS.'}
          </span>
        </div>
      </div>
    );
  }

  if (state.state === 'decision') {
    const unsent = unsentCopy(state.unsent);
    return (
      <div
        className="exit-takeover"
        role="dialog"
        aria-modal="true"
        aria-labelledby="exit-decision-title"
      >
        <div className="exit-card decision">
          <div className="exit-card-body">
            <span className="exit-card-icon">
              <Icon name="settings" />
            </span>
            <div>
              <h2 id="exit-decision-title">Stop FOKS Agent and quit?</h2>
              {unsent ? <p className="exit-unsent">{unsent}</p> : null}
              <p>
                FOKS will stop the agents using this app’s local state,
                including an agent already running when the app opened. Once
                shutdown starts, the app stays here until those processes have
                exited.
              </p>
            </div>
          </div>
          {failure ? (
            <p className="action-error" role="alert">
              {failure}
            </p>
          ) : null}
          <div className="exit-actions">
            <Button
              disabled={sending}
              data-dialog-autofocus="true"
              onClick={() => act('cancel')}
            >
              Keep FOKS open
            </Button>
            <Button
              variant="primary"
              disabled={sending}
              onClick={() => act('stop-agent')}
            >
              Quit and stop agent
            </Button>
          </div>
        </div>
      </div>
    );
  }

  if (state.state === 'failed') {
    return (
      <div
        className="exit-takeover"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="exit-failed-title"
      >
        <div className="exit-card failure">
          <div className="exit-card-body">
            <span className="exit-card-icon danger">
              <Icon name="alert" />
            </span>
            <div>
              <h2 id="exit-failed-title">FOKS Agent did not stop</h2>
              <p>
                FOKS has not quit because its background agent is still running.
                You can try again or force it to stop. The app cannot resume
                while shutdown is in progress.
              </p>
              {state.pid !== null ? (
                <div className="exit-process">
                  <span>Agent at shutdown</span>
                  <code>foks-agent (PID {state.pid})</code>
                </div>
              ) : null}
              <p className="exit-error">{state.error}</p>
            </div>
          </div>
          {failure ? (
            <p className="action-error" role="alert">
              {failure}
            </p>
          ) : null}
          <div className="exit-actions failure-actions">
            <Button danger disabled={sending} onClick={() => act('show-force')}>
              Force stop…
            </Button>
            <span className="spacer" />
            <Button
              variant="primary"
              disabled={sending}
              onClick={() => act('retry')}
            >
              Try again
            </Button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      className="exit-takeover"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="exit-force-title"
    >
      <div className="exit-card force">
        <div className="exit-card-body">
          <span className="exit-card-icon danger">
            <Icon name="alert" />
          </span>
          <div>
            <h2 id="exit-force-title">Force stop FOKS Agent?</h2>
            <p>
              The agent did not respond to a normal shutdown. Force stopping it
              can interrupt a local operation.
            </p>
            <p className="exit-calm">
              An interrupted operation may need to be checked or retried after
              FOKS starts again.
            </p>
          </div>
        </div>
        {failure ? (
          <p className="action-error" role="alert">
            {failure}
          </p>
        ) : null}
        <div className="exit-actions">
          <span className="spacer" />
          <Button
            disabled={sending}
            data-dialog-autofocus="true"
            onClick={() => act('cancel-force')}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            danger
            disabled={sending}
            onClick={() => act('force-stop')}
          >
            Force stop and quit
          </Button>
        </div>
      </div>
    </div>
  );
}

export function ExitGuard({
  bridge,
  children,
}: {
  bridge: Bridge | null;
  children?: ReactNode;
}): ReactNode {
  const [state, setState] = useState<ExitState>(IDLE);
  const [lastBridge, setLastBridge] = useState(bridge);
  const exitBridge = bridge ?? lastBridge;

  useEffect(() => {
    if (bridge?.native) setLastBridge(bridge);
  }, [bridge]);

  useEffect(() => {
    if (!exitBridge?.native) return;
    const update = (next: ExitState) =>
      setState((previous) => {
        const teardown =
          previous.state !== 'idle' && previous.state !== 'decision';
        return teardown && (next.state === 'idle' || next.state === 'decision')
          ? previous
          : next;
      });
    let live = true;
    let unlisten: (() => void) | undefined;
    let receivedEvent = false;
    void (async () => {
      const release = await exitBridge.onExitState((next) => {
        receivedEvent = true;
        if (live) update(next);
      });
      if (!live) {
        release();
        return;
      }
      unlisten = release;
      const current = await exitBridge.exitState();
      if (live && !receivedEvent) update(current);
    })().catch(() => undefined);
    return () => {
      live = false;
      unlisten?.();
    };
  }, [exitBridge]);

  const blocked = state.state !== 'idle';
  return (
    <div className="exit-guard-root">
      <div
        className="exit-guard-background"
        inert={blocked || undefined}
        aria-hidden={blocked || undefined}
      >
        {state.state === 'idle' || state.state === 'decision' ? children : null}
      </div>
      {blocked && exitBridge ? (
        <ExitOverlay state={state} bridge={exitBridge} />
      ) : null}
    </div>
  );
}
