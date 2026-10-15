import type { ReactNode } from 'react';
import type { FirstRunStateName } from '../../first-run-state';
import { presentFirstRunFailure } from '../../first-run-failure';
import { useToast } from '../../../kit/toasts';
import { LocalCompleteStep } from '../first-run-complete-step';
import { RecoveryStep } from '../first-run-recovery-step';
import { ServerVerificationStep } from '../first-run-server-step';
import { AccountStep } from './account-step';
import { IdentityPendingStep } from './identity-pending-step';
import { LocalServerStep } from './local-server-step';
import { OperationPendingStep } from './operation-pending-step';
import { ServerAddressStep } from './server-address-step';
import { SetupChecklistStep } from './setup-checklist-step';
import { setupMutating } from './setup-busy';
import { useSetupContext } from './setup-context';
import { TeamAddedStep } from './team-added-step';
import { WaitingForTeamStep } from './waiting-for-team-step';
import { WhoStep } from './who-step';

function ServerAddress(): ReactNode {
  const toasts = useToast();
  const { checkpoint, busy, message, go, server, stores, bridge } =
    useSetupContext();
  if (
    checkpoint.state !== 'address' &&
    checkpoint.state !== 'no-address' &&
    checkpoint.state !== 'error'
  )
    return null;
  return (
    <ServerAddressStep
      state={checkpoint.state}
      path={checkpoint.path}
      busy={busy !== null}
      adminShort={stores.adminShort}
      address={server.address}
      addressInvalid={server.addressInvalid}
      inputRef={server.serverAddressInput}
      serverCheckPresentation={
        server.serverCheckFailure
          ? presentFirstRunFailure(server.serverCheckFailure)
          : null
      }
      message={message}
      go={go}
      checkServer={server.checkServer}
      editServerAddress={server.editServerAddress}
      onCopy={(value) =>
        void bridge
          .copyText(value)
          .then(() => toasts.show('Copied to clipboard.'))
      }
    />
  );
}

function ServerVerification(): ReactNode {
  const { checkpoint, busy, go, server } = useSetupContext();
  if (checkpoint.state !== 'checked' && checkpoint.state !== 'compare')
    return null;
  return (
    <ServerVerificationStep
      checkpoint={checkpoint}
      state={checkpoint.state}
      profile={checkpoint.profile}
      address={server.address}
      busy={busy !== null}
      inputRef={server.serverAddressInput}
      onBack={() => go('address')}
      onContinue={() => go(checkpoint.returning ? 'existing' : 'account')}
      onCheck={() => void server.checkServer()}
      onAddressChange={server.editServerAddress}
    />
  );
}

function Protection(): ReactNode {
  const { checkpoint, busy, message, go, secrets, protection } =
    useSetupContext();
  if (checkpoint.state !== 'protect' && checkpoint.state !== 'phrase')
    return null;
  const { backup } = secrets;
  return (
    <RecoveryStep
      state={checkpoint.state}
      checkpoint={checkpoint}
      busy={busy !== null}
      recoveryPhrase={backup.generatedRecoveryPhrase}
      phraseConcealed={backup.recoveryPhraseConcealed}
      revealPhrase={() => backup.setRecoveryPhraseConcealed(false)}
      phraseWritten={backup.phraseWritten}
      setPhraseWritten={backup.setPhraseWritten}
      go={go}
      finishLocalProtection={protection.finishLocalProtection}
      collapseLocalBackup={protection.collapseLocalBackup}
      message={message}
      passphrase={secrets.passphrase}
      confirmation={secrets.confirmation}
      setPassphrase={secrets.setPassphrase}
      setConfirmation={secrets.setConfirmation}
      continueProtection={protection.continueProtection}
      mutationBusy={setupMutating(busy)}
      commitBackup={protection.commitBackup}
    />
  );
}

function LocalComplete(): ReactNode {
  const { checkpoint, snapshot, onNavigate, stores, personal } =
    useSetupContext();
  return (
    <LocalCompleteStep
      personalAvailable={stores.personalAvailable}
      accountStoreRecord={stores.accountStoreRecord}
      accountStore={stores.accountStore}
      personalRefreshing={personal.refreshing}
      retryPersonal={personal.retry}
      onNavigate={onNavigate}
      profile={checkpoint.profile}
      snapshot={snapshot}
      accountItemCount={stores.accountItemCount}
      personalRefreshError={personal.error}
    />
  );
}

function WaitingForTeam(): ReactNode {
  const toasts = useToast();
  const {
    bridge,
    checkpoint,
    commit,
    busy,
    message,
    setMessage,
    go,
    stores,
    team,
  } = useSetupContext();
  return (
    <WaitingForTeamStep
      checkpoint={checkpoint}
      busy={busy !== null}
      message={message}
      admin={stores.admin}
      adminShort={stores.adminShort}
      group={stores.group}
      discoveredGroups={team.discoveredGroups}
      discoveryOutcome={team.outcome}
      discover={team.discover}
      selectDiscoveredGroup={team.selectDiscoveredGroup}
      go={go}
      onChooseAnotherTeam={() => {
        commit({
          ...checkpoint,
          selectedGroup: undefined,
          group: undefined,
          added: false,
        });
        setMessage(null);
      }}
      onCopy={(value) =>
        void bridge
          .copyText(value)
          .then(() => toasts.show('Copied to clipboard.'))
      }
    />
  );
}

function SetupChecklist(): ReactNode {
  const toasts = useToast();
  const { bridge, checkpoint, onNavigate, go, stores, personal } =
    useSetupContext();
  return (
    <SetupChecklistStep
      checkpoint={checkpoint}
      personalRefreshError={personal.error}
      admin={stores.admin}
      adminShort={stores.adminShort}
      group={stores.group}
      accountStore={stores.accountStore}
      personalRefreshing={personal.refreshing}
      retryPersonal={personal.retry}
      onNavigate={onNavigate}
      go={go}
      onCopy={(value) =>
        void bridge.copyText(value).then(() => toasts.show('Message copied.'))
      }
    />
  );
}

/**
 * The step each checkpoint state renders. The record names every state, so a
 * state added to the machine does not compile until it has a step. States that
 * share a page share a component, so moving between them keeps the page and
 * what is typed into it mounted.
 */
export const FIRST_RUN_STEPS: {
  readonly [S in FirstRunStateName]: () => ReactNode;
} = {
  boot: WhoStep,
  who: WhoStep,
  local: LocalServerStep,
  address: ServerAddress,
  'no-address': ServerAddress,
  error: ServerAddress,
  checked: ServerVerification,
  compare: ServerVerification,
  account: AccountStep,
  existing: AccountStep,
  'identity-pending': IdentityPendingStep,
  'operation-pending': OperationPendingStep,
  protect: Protection,
  phrase: Protection,
  waiting: WaitingForTeam,
  added: TeamAddedStep,
  'local-done': LocalComplete,
  'checklist-invited': SetupChecklist,
  'checklist-own': SetupChecklist,
};
