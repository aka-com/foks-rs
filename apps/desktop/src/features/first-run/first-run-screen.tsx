import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button } from '../../components';
import type { FirstRunCheckpoint } from '../../first-run-state';
import { NavigationPrompt } from '../../navigation-guard';
import {
  AddedDetails,
  FirstRunAppSidebar,
  Pane,
  SetupSidebar,
} from './first-run-view';
import {
  SetupContext,
  type FirstRunExperienceProps,
  type FirstRunSessionProps,
} from './setup-context';
import { FIRST_RUN_STEPS } from './steps';
import { useFirstRunSession } from './use-first-run-session';
export { FirstRunChecklistStatus } from './first-run-view';
export { profileNameFor } from './use-server-workflow';
export { accountAliasFor } from './use-setup-identity';
export type { FirstRunExperienceProps } from './setup-context';

export function FirstRunExperience(props: FirstRunExperienceProps): ReactNode {
  const [session, setSession] = useState(0);
  const [sessionEntry, setSessionEntry] = useState<FirstRunCheckpoint | null>(
    null,
  );
  return (
    <FirstRunSession
      key={session}
      sessionEntry={sessionEntry}
      {...props}
      onReplaceSession={(next) => {
        setSessionEntry(next);
        props.onNavigate({
          kind: 'first-run',
          step: next.state,
          path: next.path,
        });
        setSession((value) => value + 1);
      }}
    />
  );
}

/**
 * One setup session: it provides the session to the step its checkpoint
 * names, and draws the sidebar, the main frame and the readiness blocker
 * around that step.
 */
function FirstRunSession(props: FirstRunSessionProps): ReactNode {
  const {
    location,
    onNavigate,
    agent = 'ready',
    blocked = false,
    collapsed = false,
    onToggleCollapsed,
    devicesAlert = null,
  } = props;
  const session = useFirstRunSession(props);
  const {
    bridge,
    snapshot,
    agentReady,
    checkpoint,
    send,
    go,
    busy,
    slow,
    leaveBlocked,
    secrets,
    server,
    restart,
    stores,
    joining,
    agentRetry,
  } = session;
  const state = checkpoint.state;
  const Step = FIRST_RUN_STEPS[state];
  const phraseOperation = secrets.backup.phraseOperation;
  const appMode = ['added', 'checklist-invited', 'checklist-own'].includes(
    state,
  );
  // A managed local server offers its alternatives in the sidebar until an
  // account exists or is being set up.
  const managedAlternatives =
    busy === null &&
    checkpoint.managedLocal &&
    !checkpoint.account &&
    !checkpoint.provisioning &&
    !checkpoint.provisionedAccount;
  const readinessBlocker =
    !agentReady && agentRetry.retry ? (
      <Pane title="Service setup" header={false}>
        <h1>Finish preparing this device</h1>
        <p className="lead">
          FOKS needs to finish preparing and checking local setup before account
          setup can continue. Your progress is still saved.
        </p>
        {agentRetry.error ? (
          <div className="crit" role="alert">
            <b>Setup could not be completed</b>
            {agentRetry.error}
          </div>
        ) : null}
        <div className="setup-actions">
          <Button
            variant="primary"
            disabled={agentRetry.retrying}
            onClick={agentRetry.retry}
          >
            {agentRetry.retrying ? 'Preparing service…' : 'Retry setup'}
          </Button>
        </div>
      </Pane>
    ) : null;
  return (
    <SetupContext.Provider value={session}>
      {appMode ? (
        <FirstRunAppSidebar
          snapshot={snapshot}
          native={Boolean(bridge.native)}
          checkpoint={checkpoint}
          groupName={checkpoint.group?.name ?? stores.group}
          location={location}
          onNavigate={onNavigate}
          onReenter={restart.reenterSetup}
          collapsed={collapsed}
          onToggleCollapsed={onToggleCollapsed}
          agent={agent}
          blocked={blocked}
          devicesAlert={devicesAlert}
        />
      ) : (
        <SetupSidebar
          checkpoint={checkpoint}
          native={Boolean(bridge.native)}
          agent={agent}
          blocked={blocked}
          pendingPath={joining.path}
          onAnotherServer={
            managedAlternatives
              ? () => send({ type: 'choose', path: 'own' })
              : undefined
          }
          onRecoverAccount={
            managedAlternatives
              ? () => server.selectManagedProfile(true)
              : undefined
          }
          recoverEnabled={Boolean(server.managedReport)}
          cancelDisabled={leaveBlocked}
          onRestart={() => restart.setConfirmingRestart(true)}
          restartDisabled={!restart.recoveryActions.canRestart}
          restartReason={restart.recoveryActions.restartReason}
          onCancel={() => {
            // Clear entered values before navigating so the guard allows the
            // exit.
            secrets.clear();
            if (state === 'phrase')
              send({ type: 'navigate', state: 'protect' });
            onNavigate({ kind: 'all' });
          }}
        />
      )}
      <main
        className="main first-run-main"
        aria-busy={agentReady && busy !== null}
      >
        {restart.restartError ? (
          <p role="alert" className="crit">
            {restart.restartError}
          </p>
        ) : null}
        {slow || phraseOperation ? (
          <div className="bandstrip">
            <div className="band" role="status">
              <span className="t">
                {slow
                  ? `This is taking longer than expected. ${
                      checkpoint.provisioning
                        ? 'You can finish later while account setup completes.'
                        : 'FOKS is still waiting for a response.'
                    }`
                  : 'Preparing your recovery phrase…'}
              </span>
              {phraseOperation ? (
                <span className="a">
                  <Button
                    onClick={() => {
                      secrets.clear();
                      go('protect');
                    }}
                  >
                    Back to recovery options
                  </Button>
                </span>
              ) : null}
            </div>
          </div>
        ) : null}
        <div
          data-setup-controls=""
          style={{ display: 'contents' }}
          inert={agentReady && busy !== null && !checkpoint.provisioning}
        >
          {readinessBlocker ?? <Step />}
        </div>
      </main>
      {state === 'added' ? (
        <AddedDetails snapshot={snapshot} storeId={stores.addedStore} />
      ) : null}
      {restart.confirmingRestart ? (
        <NavigationPrompt
          verdict={{
            verdict: 'prompt',
            title: 'Restart setup?',
            body: 'The setup in progress on this device will be discarded. If an account was already created on the server, you will need a recovery key to reconnect to it.',
            confirm: 'Start over',
          }}
          onConfirm={restart.startSetupOver}
          onCancel={() => restart.setConfirmingRestart(false)}
        />
      ) : null}
    </SetupContext.Provider>
  );
}
