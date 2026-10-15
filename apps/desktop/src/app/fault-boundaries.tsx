/**
 * Error boundaries for the parts of the window outside the routed screen.
 *
 * `PanelErrorBoundary` wraps a panel or dialog the shell can close. A render
 * error unmounts the panel, which discards any value it had revealed, and the
 * shell closes it; the rail, topbar and screen remain. `WindowFaultBoundary`
 * replaces everything below it with a fault screen whose actions use no shell
 * context: Reload window, Copy diagnostics and Quit FOKS. Both record the
 * fault in the timing log under their static label (see
 * `diagnostics/renderer-fault.ts`), so Copy diagnostics reports it.
 */

import { Component, useEffect, useRef, useState } from 'react';
import type { ErrorInfo, ReactNode } from 'react';
import { normalizeCommandError, type Bridge } from '../bridge';
import { Button } from '../components';
import { formatTimings } from '../diagnostics/format';
import { diagnosticLog, type TimingEvent } from '../diagnostics/log';
import {
  recordRendererFault,
  type LabelledBoundary,
} from '../diagnostics/renderer-fault';
import { backendTimingSource } from '../diagnostics/subscribe';
import { useCopyDiagnostics } from '../diagnostics/use-copy-diagnostics';

interface PanelErrorBoundaryProps {
  /** The static label the fault is recorded under. */
  label: string;
  /** Closes the panel. Called once per fault, after it has unmounted. */
  onFault?: () => void;
  /**
   * The panel's open state or contents; a failure is cleared when it changes,
   * so a panel that stays mounted while closed can open again.
   */
  identity?: string;
  children: ReactNode;
}

export class PanelErrorBoundary
  extends Component<PanelErrorBoundaryProps, { failed: boolean }>
  implements LabelledBoundary
{
  override state = { failed: false };

  get faultLabel(): string {
    return this.props.label;
  }

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }

  override componentDidUpdate(previous: PanelErrorBoundaryProps): void {
    if (this.state.failed && previous.identity !== this.props.identity)
      this.setState({ failed: false });
  }

  override componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error(
      `The ${this.props.label} panel failed to render.`,
      error,
      info.componentStack,
    );
    recordRendererFault(this.props.label, error);
    this.props.onFault?.();
  }

  override render(): ReactNode {
    return this.state.failed ? null : this.props.children;
  }
}

/**
 * The text the fault screen's Copy diagnostics puts on the clipboard: the
 * timing log as the Refresh status popover formats it, without the status
 * lines that need the shell's snapshot.
 */
export function faultDiagnosticsText(
  events: readonly TimingEvent[],
  now: number = Date.now(),
): string {
  return formatTimings(events, {
    now,
    header: [
      `renderer fault screen · window ${
        typeof document === 'undefined' ? 'unknown' : document.visibilityState
      }`,
    ],
  });
}

type FaultBridge = Pick<
  Bridge,
  'native' | 'quitApp' | 'agentProcessInfo' | 'appInfo' | 'agentStatus'
> &
  Pick<Partial<Bridge>, 'diagnosticTimings' | 'probeAgentStatus'>;

/** The backend's events, through the bridge; none before boot. */
async function backendEvents(
  bridge: FaultBridge | null,
): Promise<readonly TimingEvent[]> {
  const source = bridge ? backendTimingSource(bridge) : undefined;
  const backend = source
    ? await source().catch((): readonly TimingEvent[] => [])
    : [];
  return backend.filter((event) => event.layer !== 'renderer');
}

/** The renderer's events merged with `backend`, ordered by time. */
function withRendererEvents(backend: readonly TimingEvent[]): TimingEvent[] {
  return [...diagnosticLog.events(), ...backend].sort((a, b) => a.at - b.at);
}

export function RendererFaultScreen({
  error,
  bridge,
}: {
  error: unknown;
  bridge: FaultBridge | null;
}): ReactNode {
  const [failure, setFailure] = useState<string | null>(null);
  // As in the Refresh status popover, the backend's events are read once on
  // showing, for clipboards that cannot wait for a fresh read.
  const [backend, setBackend] = useState<readonly TimingEvent[]>([]);
  useEffect(() => {
    let current = true;
    void backendEvents(bridge).then((events) => {
      if (current) setBackend(events);
    });
    return () => {
      current = false;
    };
  }, [bridge]);
  const { copied, copy } = useCopyDiagnostics(
    () =>
      backendEvents(bridge).then((latest) =>
        faultDiagnosticsText(withRendererEvents(latest)),
      ),
    () => faultDiagnosticsText(withRendererEvents(backend)),
  );
  // The failed tree took the focused element with it. Focus moves to the
  // card's primary action so keyboard and screen reader users reach it.
  const reload = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    reload.current?.focus({ preventScroll: true });
  }, []);
  return (
    <div
      className={[
        'window',
        bridge?.native ? 'native-window' : 'web-mock-window',
      ].join(' ')}
    >
      <div
        className="takeover stopwrap fault-takeover"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="renderer-fault-title"
        data-tauri-drag-region
      >
        <div className="tcard stopcard">
          <h2 id="renderer-fault-title">This window could not be shown</h2>
          <p>
            An error occurred while drawing this window. Reload the window to
            open it again. Your vaults remain on this device and on their
            configured servers.
          </p>
          <p className="fn bad">{normalizeCommandError(error).message}</p>
          {failure ? (
            <p className="fn bad" role="alert">
              {failure}
            </p>
          ) : null}
          <div className="tacts">
            <Button
              ref={reload}
              variant="primary"
              onClick={() => window.location.reload()}
            >
              Reload window
            </Button>
            <Button icon={copied ? 'check' : undefined} onClick={copy}>
              {copied ? 'Copied' : 'Copy diagnostics'}
            </Button>
            {bridge ? (
              <Button
                onClick={() => {
                  setFailure(null);
                  void bridge
                    .quitApp()
                    .catch((reason) =>
                      setFailure(normalizeCommandError(reason).message),
                    );
                }}
              >
                Quit FOKS
              </Button>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}

interface WindowFaultBoundaryProps {
  /** The static label the fault is recorded under. */
  label: string;
  /** Supplies Quit FOKS and the backend's timings; absent before boot. */
  bridge?: FaultBridge | null;
  children: ReactNode;
}

export class WindowFaultBoundary
  extends Component<
    WindowFaultBoundaryProps,
    { failure: { error: unknown } | null }
  >
  implements LabelledBoundary
{
  override state: { failure: { error: unknown } | null } = { failure: null };

  get faultLabel(): string {
    return this.props.label;
  }

  static getDerivedStateFromError(error: unknown): {
    failure: { error: unknown };
  } {
    return { failure: { error } };
  }

  override componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error('The window failed to render.', error, info.componentStack);
    recordRendererFault(this.props.label, error);
  }

  override render(): ReactNode {
    const { failure } = this.state;
    return failure ? (
      <RendererFaultScreen
        error={failure.error}
        bridge={this.props.bridge ?? null}
      />
    ) : (
      this.props.children
    );
  }
}
