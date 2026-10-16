/** Settings › Device: the application, its agent, local data and reset. */

import type { CollectionReadiness } from '../../model';
import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import type { AgentProcessInfo, AppInfo, Bridge } from '../../bridge';
import { Button, Inset, InsetRow, SectionLabel } from '../../components';
import type { AgentSnapshot } from '../../model';
import {
  agentLifecycleLabel,
  type AgentLifecycle,
} from '../../agent-lifecycle';
import { isExternalAgent, RestartAgentSheet } from './restart-agent-sheet';
import type { RestartPurpose, RestartRequest } from './restart-agent-sheet';

/**
 * Device: what the application and its agent are, the local FOKS data
 * operations, and the one reset that acts on this Mac rather than on an
 * account. The reset stays visually marked as destructive, in its own danger
 * zone at the foot of this page rather than of everything Settings holds.
 */
export function DeviceSection({
  snapshot,
  bridge,
  appInfo,
  onError,
  onMessage,
  onLock,
  agentLifecycle,
  onRetryAgent,
  serversCount,
  onReset,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  appInfo: AppInfo | null;
  onError: (error: unknown) => void;
  onMessage: (message: string) => void;
  onLock: () => Promise<boolean>;
  agentLifecycle: AgentLifecycle;
  onRetryAgent: () => Promise<void>;
  serversCount: number;
  onReset: () => void;
}): ReactNode {
  const ready =
    snapshot.agent.state === 'ready' && agentLifecycle.state === 'ready';
  const maintenanceUnavailable = agentLifecycle.state !== 'ready';
  // The process on the socket: who started it decides whether maintenance can
  // stop it, and Status says so. Re-read whenever the agent's state settles.
  const [process, setProcess] = useState<AgentProcessInfo | null>(null);
  const [processReadiness, setProcessReadiness] =
    useState<CollectionReadiness>('loading');
  const [processAttempt, setProcessAttempt] = useState(0);
  useEffect(() => {
    let alive = true;
    setProcess(null);
    if (!ready) {
      setProcessReadiness('unavailable');
      return;
    }
    setProcessReadiness('loading');
    void bridge
      .agentProcessInfo()
      .then((info) => {
        if (alive) {
          setProcess(info);
          setProcessReadiness('ready');
        }
      })
      .catch(() => {
        if (alive) setProcessReadiness('unavailable');
      });
    return () => {
      alive = false;
    };
  }, [bridge, ready, agentLifecycle.state, processAttempt]);
  const foreign = process !== null && process.pid !== null && !process.owned;
  // The restart sheet: opened from the Status row, or by a maintenance action
  // that the agent's ownership refused, in which case it carries the action
  // to run once the agent is ours.
  const [restart, setRestart] = useState<RestartRequest | null>(null);
  const maintain = (
    purpose: RestartPurpose,
    run: () => Promise<unknown>,
  ): void => {
    void run().catch((error: unknown) => {
      if (isExternalAgent(error)) setRestart({ purpose, then: run });
      else onError(error);
    });
  };
  return (
    <>
      {restart ? (
        <RestartAgentSheet
          bridge={bridge}
          request={restart}
          process={process}
          processReadiness={processReadiness}
          onRetryInfo={() => setProcessAttempt((attempt) => attempt + 1)}
          onClose={() => setRestart(null)}
          onError={onError}
        />
      ) : null}
      <SectionLabel>Application</SectionLabel>
      <Inset className="settings-inset middle wide">
        <InsetRow label="Version">
          FOKS Desktop {appInfo?.version ?? '…'}
        </InsetRow>
        <InsetRow
          label="Lock"
          action={
            <Button
              size="sm"
              icon="shield"
              onClick={() => {
                void onLock().then(
                  (locked) => {
                    if (!locked)
                      onMessage(
                        'Application lock is not available on this system.',
                      );
                  },
                  (error) => onError(error),
                );
              }}
            >
              Lock now
            </Button>
          }
        >
          Require an operating-system unlock before FOKS can read vault data
          again.
        </InsetRow>
      </Inset>
      <SectionLabel>Agent</SectionLabel>
      {/* Top-align the multi-line Status row with its primary status line.
          Single-line Socket rows use vertical centering. */}
      <Inset className="settings-inset wide">
        <InsetRow
          label="Status"
          action={
            ready ? (
              <Button
                size="sm"
                disabled={maintenanceUnavailable}
                onClick={() => setRestart({ purpose: 'restart' })}
              >
                Restart…
              </Button>
            ) : (
              <Button
                size="sm"
                variant="primary"
                onClick={() =>
                  void onRetryAgent()
                    .then(() => onMessage('Connected to local agent.'))
                    .catch(onError)
                }
              >
                Retry connection
              </Button>
            )
          }
        >
          <span className={ready ? 'agent' : 'agent warn'}>
            <i />
            {snapshot.agent.state === 'bootstrap'
              ? 'Starting the FOKS agent'
              : agentLifecycleLabel(agentLifecycle)}
          </span>
          {ready ? null : (
            <small>
              The local background agent must be connected to use FOKS.
            </small>
          )}
          {foreign ? (
            <small>
              Agent started by another app. Use 'Restart...' to take ownership
              of the agent.
            </small>
          ) : null}
        </InsetRow>
        <InsetRow
          className="line"
          label="Socket"
          valueClass="mono"
          action={
            appInfo ? (
              <Button
                size="sm"
                onClick={() =>
                  void bridge
                    .copyText(appInfo.agentSocket)
                    .then(() => onMessage('Socket path copied'))
                    .catch(onError)
                }
              >
                Copy
              </Button>
            ) : undefined
          }
        >
          {appInfo?.agentSocket ?? 'Reading app info…'}
        </InsetRow>
      </Inset>
      <SectionLabel>FOKS data</SectionLabel>
      <Inset className="settings-inset wide">
        <InsetRow
          label="Move data"
          action={
            <Button
              size="sm"
              disabled={maintenanceUnavailable}
              onClick={() =>
                maintain('relocate', () => bridge.relocateClientState())
              }
            >
              Choose folder…
            </Button>
          }
        >
          <small>
            Move every profile and its credentials to another folder. Requires
            restart.
          </small>
        </InsetRow>
        <InsetRow
          label="Transfer"
          action={
            <>
              <Button
                size="sm"
                disabled={maintenanceUnavailable}
                onClick={() =>
                  maintain('export', () => bridge.maintainClientState('export'))
                }
              >
                Export…
              </Button>
              <Button
                size="sm"
                disabled={maintenanceUnavailable}
                onClick={() =>
                  maintain('import', () => bridge.maintainClientState('import'))
                }
              >
                Import…
              </Button>
            </>
          }
        >
          <small>
            Export this app's accounts. Clones the device; we recommend linking
            a new device instead.
          </small>
        </InsetRow>
        <InsetRow
          label="Verify all servers"
          action={
            <Button
              size="sm"
              disabled={maintenanceUnavailable}
              onClick={() =>
                maintain('verify', () => bridge.maintainClientState('verify'))
              }
            >
              Verify online
            </Button>
          }
        >
          <small>
            Check the current account is still valid on all connected servers.
          </small>
        </InsetRow>
      </Inset>
      <SectionLabel className="danger-title">Danger zone</SectionLabel>
      <Inset className="settings-inset middle wide danger-box">
        <InsetRow
          className="dangerrow"
          label="Reset this device"
          action={
            <Button
              size="sm"
              variant="danger"
              disabled={!serversCount}
              title={
                serversCount
                  ? undefined
                  : 'No server is configured on this device'
              }
              onClick={onReset}
            >
              Reset…
            </Button>
          }
        >
          <small>
            Deletes local account keys, trust history, cached state, and pending
            operations.
          </small>
        </InsetRow>
      </Inset>
    </>
  );
}
