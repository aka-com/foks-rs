import { useState, type ReactNode } from 'react';
import { Dialog, useHasOverlayProvider } from '/kit/overlay-primitives';
import { normalizeCommandError, type Bridge } from '../bridge';
import {
  agentLifecycleLabel,
  maintenanceOutcomeMessage,
  type AgentLifecycle,
} from '../agent-lifecycle';
import { Button, CopyBox, Icon } from '../components';
import type { AgentSnapshot } from '../model';
import type { BootProgress } from './app-bootstrap';
import { Sidebar, railAgentState, type RailAgentState } from '../shell/sidebar';
import { Topbar } from '../shell/topbar';

type AgentStop = Extract<
  AgentLifecycle,
  {
    state:
      | 'maintenance'
      | 'restart-required'
      | 'recovery-required'
      | 'restoration-failed';
  }
>;

function recoveryCommands(root: string): readonly string[] {
  return [
    `foks-rs --state-dir ${root} state status`,
    `foks-rs --state-dir ${root} state recover`,
  ];
}

function agentStopDetail(lifecycle: AgentStop): string {
  switch (lifecycle.state) {
    case 'maintenance':
      return 'FOKS is paused while background maintenance completes.';
    case 'recovery-required':
      return `The data directory at ${lifecycle.root} requires recovery. Run the following commands before reopening FOKS:`;
    case 'restart-required':
      return `Please restart FOKS to open ${lifecycle.root}.`;
    case 'restoration-failed':
      return lifecycle.error.message;
  }
}

/** The lifecycle as a stop state, or null while the agent runs or starts. */
function agentStop(lifecycle: AgentLifecycle): AgentStop | null {
  switch (lifecycle.state) {
    case 'maintenance':
    case 'restart-required':
    case 'recovery-required':
    case 'restoration-failed':
      return lifecycle;
    default:
      return null;
  }
}

export type ShellBlock =
  | { kind: 'starting'; progress?: BootProgress }
  | { kind: 'locked' }
  | { kind: 'boot-error'; message: string }
  | { kind: 'stop'; lifecycle: AgentStop }
  | { kind: 'disconnected'; message?: string; credentialsRequired?: boolean };

export function shellBlock(
  lifecycle: AgentLifecycle,
  startup?: {
    locked: boolean;
    error: string | null;
    pending: boolean;
    /** The catalog read behind the loading screen, once it reports. */
    progress?: BootProgress | null;
  },
): ShellBlock | null {
  if (startup) {
    if (startup.locked) return { kind: 'locked' };
    if (startup.error !== null)
      return { kind: 'boot-error', message: startup.error };
    if (!startup.pending) return null;
    const stop = agentStop(lifecycle);
    return stop && stop.state !== 'maintenance'
      ? { kind: 'stop', lifecycle: stop }
      : {
          kind: 'starting',
          ...(startup.progress ? { progress: startup.progress } : {}),
        };
  }
  if (
    lifecycle.state === 'failure' &&
    normalizeCommandError(lifecycle.error).code === 'agent-credentials-required'
  )
    return {
      kind: 'disconnected',
      credentialsRequired: true,
      message: normalizeCommandError(lifecycle.error).message,
    };
  const stop = agentStop(lifecycle);
  if (stop) return { kind: 'stop', lifecycle: stop };
  return lifecycle.state === 'disconnected'
    ? { kind: 'disconnected', message: lifecycle.error }
    : null;
}

export function shellChrome(
  block: ShellBlock | null,
  lifecycle: AgentLifecycle['state'] = 'ready',
  snapshotAgent?: AgentSnapshot['agent']['state'],
): { blocked: boolean; agent: RailAgentState } {
  return {
    blocked: block !== null,
    agent: !block
      ? railAgentState(lifecycle, snapshotAgent)
      : block.kind === 'locked'
        ? 'locked'
        : block.kind === 'starting' ||
            (block.kind === 'stop' && block.lifecycle.state === 'maintenance')
          ? 'starting'
          : 'stopped',
  };
}

export function AgentStopCard({
  lifecycle,
  bridge,
  onRetryRestoration,
}: {
  lifecycle: AgentStop;
  bridge: Bridge;
  onRetryRestoration: () => void;
}): ReactNode {
  const [failure, setFailure] = useState<string | null>(null);
  const report = (error: unknown): void =>
    setFailure(normalizeCommandError(error).message);
  const label = agentLifecycleLabel(lifecycle);
  const quit = (
    <Button
      onClick={() => {
        void bridge.quitApp().catch(report);
      }}
    >
      Quit FOKS
    </Button>
  );
  const actions =
    lifecycle.state === 'restart-required' ? (
      // A blocking restart state provides both restart and quit actions.
      <>
        <Button
          variant="primary"
          onClick={() => {
            void bridge.restartApp().catch(report);
          }}
        >
          Restart FOKS
        </Button>
        {quit}
      </>
    ) : lifecycle.state === 'recovery-required' ? (
      quit
    ) : lifecycle.state === 'restoration-failed' ? (
      <>
        <Button variant="primary" onClick={onRetryRestoration}>
          Restart service
        </Button>
        {quit}
      </>
    ) : null;
  const body = (
    <>
      {lifecycle.state !== 'maintenance' ? (
        <p>{maintenanceOutcomeMessage(lifecycle.operation)}</p>
      ) : null}
      <p>{agentStopDetail(lifecycle)}</p>
      {lifecycle.state === 'recovery-required'
        ? recoveryCommands(lifecycle.root).map((command) => (
            <CopyBox
              key={command}
              text={command}
              onCopy={(text) => {
                void bridge.copyText(text).catch(report);
              }}
            >
              <code>{command}</code>
            </CopyBox>
          ))
        : null}
      {lifecycle.state === 'restart-required' ? (
        <p className="calm">
          Your vaults remain on this device and on their configured servers.
        </p>
      ) : null}
      {failure ? (
        <p className="action-error" role="alert">
          {failure}
        </p>
      ) : null}
      {actions ? <div className="acts2">{actions}</div> : null}
    </>
  );
  return (
    <div className="card stopcard">
      <h2>
        <Icon name="alert" />
        {label}
      </h2>
      {body}
    </div>
  );
}

export function AgentLostCard({
  message,
  credentialsRequired = false,
  bridge,
  onRetryAgent,
}: {
  message?: string;
  credentialsRequired?: boolean;
  bridge: Bridge;
  onRetryAgent: () => Promise<void>;
}): ReactNode {
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  return (
    <div className="notice stop">
      <h2>Connection to background service lost</h2>
      <p>
        {credentialsRequired
          ? 'Keychain access is needed to restore your connection.'
          : 'The background service stopped responding. Click Retry to reconnect.'}
      </p>
      {message && !credentialsRequired ? <p className="fn">{message}</p> : null}
      {failure ? (
        <p className="fn" role="alert">
          {failure}
        </p>
      ) : null}
      <div className="acts2">
        <Button
          variant="primary"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            setFailure(null);
            void (async () => {
              try {
                await onRetryAgent();
              } catch (error) {
                setFailure(normalizeCommandError(error).message);
              } finally {
                setBusy(false);
              }
            })();
          }}
        >
          {credentialsRequired ? 'Restore connection' : 'Retry'}
        </Button>
        <Button
          disabled={busy}
          onClick={() => {
            void bridge
              .quitApp()
              .catch((error) =>
                setFailure(normalizeCommandError(error).message),
              );
          }}
        >
          Quit FOKS
        </Button>
      </div>
    </div>
  );
}

export function Takeover({
  block,
  children,
}: {
  block: ShellBlock;
  children?: ReactNode;
}): ReactNode {
  const modal = useHasOverlayProvider();
  const identity =
    block.kind === 'stop'
      ? `${block.kind}:${block.lifecycle.state}:${block.lifecycle.generation}`
      : block.kind;
  if (block.kind === 'starting')
    return (
      <div className="takeover">
        <StartingScreen progress={block.progress} />
      </div>
    );
  const className = `takeover ${
    block.kind === 'stop'
      ? 'stopveil'
      : block.kind === 'disconnected'
        ? 'stopwrap'
        : 'lock-back'
  }`;
  const label =
    block.kind === 'stop'
      ? agentLifecycleLabel(block.lifecycle)
      : block.kind === 'disconnected'
        ? 'Connection to background service lost'
        : block.kind === 'locked'
          ? 'FOKS is locked'
          : 'Couldn’t load FOKS';
  const role = block.kind === 'locked' ? 'dialog' : 'alertdialog';
  if (modal)
    return (
      <Dialog
        key={identity}
        className={className}
        role={role}
        aria-label={label}
      >
        {children}
      </Dialog>
    );
  // Before the shell mounts there is no overlay environment to isolate the
  // background from, and the frame behind this card is already inert.
  return (
    <div className={className} role={role} aria-modal="true" aria-label={label}>
      {children}
    </div>
  );
}

/**
 * Empty shell layout rendering a dimmed, disabled rail and topbar beside the
 * takeover container. Shared by starting, stopped, and locked states to
 * preserve window geometry across transitions.
 */
export function BlockedShell({
  bridge,
  block,
  children,
}: {
  bridge?: Bridge | null;
  block: ShellBlock;
  children?: ReactNode;
}): ReactNode {
  const nowhere = (): void => undefined;
  const chrome = shellChrome(block);
  return (
    <div
      className={[
        'window',
        bridge?.native ? 'native-window' : 'web-mock-window',
      ].join(' ')}
    >
      <div className="app">
        <Sidebar
          location={{ kind: 'files' }}
          onNavigate={nowhere}
          agent={chrome.agent}
          nativeChrome={Boolean(bridge?.native)}
          blocked={chrome.blocked}
        />
        <main className="main">
          <Topbar
            location={{ kind: 'files' }}
            onNavigate={nowhere}
            collapsed={false}
            blocked={chrome.blocked}
          />
        </main>
      </div>
      <Takeover block={block}>{children}</Takeover>
    </div>
  );
}

/**
 * The content area while the agent starts and, once the catalog read reports,
 * while it runs. The live region keeps its identity across partials so the
 * progress line is re-read rather than announced as a new region.
 */
function StartingScreen({ progress }: { progress?: BootProgress }): ReactNode {
  return (
    <div className="booting" role="status">
      <span className="spin" aria-hidden="true" />
      <b>
        {progress ? 'Connecting to your vaults' : 'Starting the FOKS agent…'}
      </b>
      <span className="line">
        {progress
          ? 'This can take a few seconds.'
          : 'Startup usually takes a few seconds.'}
      </span>
      {progress && progress.total > 0 ? (
        <span className="line prog">
          {progress.ready < progress.total
            ? `${progress.ready} of ${progress.total} profiles ready`
            : 'Loading items…'}
        </span>
      ) : null}
    </div>
  );
}
